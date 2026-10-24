import 'dart:io';
import 'dart:async';
import 'dart:math';

import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../bindings/bindings.dart';
import '../../providers/cover_wall_display.dart';
import '../../utils/color_brightness.dart';
import '../../utils/rune_log.dart';
import '../../utils/visualizer_appearance.dart';
import '../../utils/visualizer_preset_dirs.dart';
import '../../utils/file_storage/mac_secure_manager.dart';

import 'visualizer_controller.dart';

const _channel = MethodChannel('not.ci.rune/visualizer');

const _targetFps = 60;
const _maxRenderWidth = 1920;
const _maxRenderHeight = 1080;
const _resizeDebounce = Duration(milliseconds: 300);

/// The physical size of the view, shrunk to fit 1920x1080 with its aspect
/// ratio kept. The width is rounded down to a multiple of 64, because
/// Windows and Linux hand the frames to Flutter without copying only when
/// the rows are tightly packed; the height follows the width so the aspect
/// ratio stays the view's, and is rounded to an even number.
(int, int) _renderSize(Size logical, double pixelRatio) {
  final width = logical.width * pixelRatio;
  final height = logical.height * pixelRatio;
  final scale = min(
    1.0,
    min(_maxRenderWidth / width, _maxRenderHeight / height),
  );
  final renderWidth = max(64, (width * scale).floor() ~/ 64 * 64);
  final renderHeight = max(2, (renderWidth * height / width).round() & ~1);
  return (renderWidth, renderHeight);
}

/// The MilkDrop visualizer, drawn into a texture that fills the available
/// space. The texture and the Rust render thread exist as long as this widget
/// is in the tree. [controller] gives the widgets around it access to the
/// presets.
class VisualizerSurface extends StatefulWidget {
  final VisualizerController controller;

  const VisualizerSurface({super.key, required this.controller});

  @override
  State<VisualizerSurface> createState() => _VisualizerSurfaceState();
}

class _VisualizerSurfaceState extends State<VisualizerSurface>
    with WidgetsBindingObserver {
  int? _textureId;
  (int, int)? _renderedSize;
  bool _starting = false;
  bool _disposed = false;

  Timer? _resizeTimer;
  Timer? _statusTimer;

  bool _firstPresetLoaded = false;
  int _pollTick = 0;

  VisualizerController get _controller => widget.controller;

  @override
  void initState() {
    super.initState();
    _controller.attach();
    WidgetsBinding.instance.addObserver(this);
    _syncVisible(WidgetsBinding.instance.lifecycleState);
    _statusTimer = Timer.periodic(
      const Duration(milliseconds: 250),
      (_) => _pollStatus(),
    );
  }

  // The appearance last sent to Rust, which picks the default preset's
  // version; null before the first build.
  bool? _light;
  // The accent colour last sent to Rust, for scenes; null before the first
  // build.
  int? _accent;
  // The preset name the corner label last announced.
  String? _announced;

  // A preset from a folder that just started gets its name shown; a problem
  // shows until it is gone; the default preset shows nothing.
  void _updateLabel(VisualizerStatusResponse status) {
    final error = status.lastError;
    if (error != null) {
      _announced = null;
      _controller.label.value = VisualizerLabel(error, isError: true);
      return;
    }
    if (!status.running || status.presetCount == 0) {
      _announced = null;
      _controller.label.value = null;
      return;
    }
    if (status.presetName == _announced) return;
    _announced = status.presetName;
    _controller.label.value = VisualizerLabel(status.presetName);
  }
  // Kept for dispose, where the context may no longer be used.
  late final CoverWallDisplayProvider _coverWallDisplay =
      context.read<CoverWallDisplayProvider>();

  void _syncAppearance(bool light) {
    if (_light == light) return;
    _light = light;
    SetVisualizerAppearanceRequest(light: light).sendSignalToRust();
  }

  void _syncAccent(Color accent) {
    final argb = accent.hexValue;
    if (_accent == argb) return;
    _accent = argb;
    SetVisualizerAccentRequest(argb: argb).sendSignalToRust();
  }

  // Whether Rust was last told the window can be seen; null until it is told.
  bool? _visible;

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) =>
      _syncVisible(state);

  // A window in the background still shows (inactive); a minimised, hidden or
  // fully covered one does not, and the visualizer stands still until it
  // shows again.
  void _syncVisible(AppLifecycleState? state) {
    final visible = switch (state) {
      AppLifecycleState.hidden ||
      AppLifecycleState.paused ||
      AppLifecycleState.detached => false,
      _ => true,
    };
    if (_visible == visible) return;
    _visible = visible;
    SetVisualizerVisibleRequest(visible: visible).sendSignalToRust();
  }

  @override
  void dispose() {
    _disposed = true;
    WidgetsBinding.instance.removeObserver(this);
    _resizeTimer?.cancel();
    _statusTimer?.cancel();
    _controller.detach();
    // Not while the tree is being torn down: listeners rebuild.
    scheduleMicrotask(() => _coverWallDisplay.presetsLoaded = false);

    if (_textureId != null || _starting) {
      StopVisualizerRequest().sendSignalToRust();
      _channel.invokeMethod<void>('dispose').catchError((Object e) {
        error$('Failed to dispose the visualizer texture: $e');
      });
    }
    super.dispose();
  }

  Future<void> _start(Size logical, double pixelRatio) async {
    if (_starting || _textureId != null) return;
    _starting = true;

    try {
      final size = _renderSize(logical, pixelRatio);

      final id = await _channel.invokeMethod<int>('create');
      if (_disposed) {
        await _channel.invokeMethod<void>('dispose');
        return;
      }

      // Bookmarks are resolved when the manager is created; wait for them
      // before Rust reads the saved folders.
      if (Platform.isMacOS) await MacSecureManager().completed;
      final dirs = await loadVisualizerPresetDirs();
      if (_disposed) {
        await _channel.invokeMethod<void>('dispose');
        return;
      }

      if (dirs.isNotEmpty) {
        SetVisualizerPresetDirsRequest(paths: dirs).sendSignalToRust();
      }
      StartVisualizerRequest(
        width: size.$1,
        height: size.$2,
        fps: _targetFps,
      ).sendSignalToRust();

      _controller.hasPresets = dirs.isNotEmpty;
      setState(() {
        _textureId = id;
        _renderedSize = size;
      });
    } catch (e) {
      error$('Failed to start the visualizer: $e');
      // Leave `_starting` set so the next frames don't retry in a loop; showing
      // the surface again retries.
      _controller.startError = e.toString();
      _controller.label.value = VisualizerLabel(e.toString(), isError: true);
    }
  }

  void _scheduleResize(Size logical, double pixelRatio) {
    if (_textureId == null) return;
    final size = _renderSize(logical, pixelRatio);
    if (size == _renderedSize) return;

    _resizeTimer?.cancel();
    _resizeTimer = Timer(_resizeDebounce, () {
      if (_disposed) return;
      ResizeVisualizerRequest(
        width: size.$1,
        height: size.$2,
      ).sendSignalToRust();
      setState(() => _renderedSize = size);
    });
  }

  Future<void> _pollStatus() async {
    if (_textureId == null || _disposed) return;

    // About 1 Hz, but 4 Hz while waiting for the render thread to come up.
    final waiting = _controller.status?.running != true;
    if (!waiting && ++_pollTick % 4 != 0) return;

    VisualizerStatusRequest().sendSignalToRust();
    try {
      final signal = await VisualizerStatusResponse.rustSignalStream.first
          .timeout(const Duration(seconds: 1));
      if (_disposed || !mounted) return;
      final status = signal.message;

      // Until the render thread is up there is nothing to send commands to.
      if (!_firstPresetLoaded && status.running) {
        // Without presets the visualizer stays on its default preset.
        if (status.presetCount > 0) {
          _firstPresetLoaded = true;
          RandomVisualizerPresetRequest(
            transitionSeconds: 0,
          ).sendSignalToRust();
        }
      }

      _controller.status = status;
      _updateLabel(status);
      // A folder without presets counts as none.
      if (status.running) {
        _controller.hasPresets = status.presetCount > 0;
        // Presets from the folder draw on black; the cover wall darkens for
        // them, but not for the default preset, which follows the app.
        context.read<CoverWallDisplayProvider>().presetsLoaded =
            status.presetCount > 0;
      }
    } on TimeoutException {
      // The next tick asks again.
    }
  }

  @override
  Widget build(BuildContext context) {
    final pixelRatio = MediaQuery.devicePixelRatioOf(context);
    final light = isAppLight(context);
    _syncAppearance(light);
    _syncAccent(FluentTheme.of(context).accentColor);

    return ColoredBox(
      // Under the letterbox bars and before the first frame, match the
      // default preset's background.
      color: light ? visualizerLightPaper : Colors.black,
      child: LayoutBuilder(
        builder: (context, constraints) {
          final size = constraints.biggest;
          if (size.width > 0 && size.height > 0) {
            WidgetsBinding.instance.addPostFrameCallback((_) {
              if (!mounted) return;
              if (_textureId == null) {
                _start(size, pixelRatio);
              } else {
                _scheduleResize(size, pixelRatio);
              }
            });
          }

          return _buildTexture();
        },
      ),
    );
  }

  /// Letterboxed: the texture keeps the aspect ratio of the size that was
  /// requested from Rust, so a window that changes shape never stretches it.
  Widget _buildTexture() {
    final id = _textureId;
    final size = _renderedSize;
    if (id == null || size == null) return const SizedBox.expand();

    return Center(
      child: AspectRatio(
        aspectRatio: size.$1 / size.$2,
        child: Texture(textureId: id),
      ),
    );
  }
}
