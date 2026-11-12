import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../bindings/bindings.dart';
import '../../providers/visualizer_scenes.dart';
import '../../utils/color_brightness.dart';
import '../../utils/rune_log.dart';
import '../../utils/api/request_failure.dart';

import 'visualizer_controller.dart';
import 'visualizer_render_size.dart';

const _targetFps = 60;
const _resizeDebounce = Duration(milliseconds: 300);

/// The visualizer, drawn into a texture that fills the available space. The
/// texture and the Rust render thread exist as long as this widget is in the
/// tree. [controller] gives the widgets around it access to the scenes.
class VisualizerSurface extends StatefulWidget {
  final VisualizerController controller;

  const VisualizerSurface({super.key, required this.controller});

  @override
  State<VisualizerSurface> createState() => _VisualizerSurfaceState();
}

class _VisualizerSurfaceState extends State<VisualizerSurface>
    with WidgetsBindingObserver {
  // The texture Rust registered and sent the id of (on Windows a new one when
  // frames start to be read back).
  int? _textureId;
  // The texture ids from Rust.
  StreamSubscription<Object?>? _textureSubscription;
  (int, int)? _renderedSize;
  bool _starting = false;
  bool _disposed = false;

  Timer? _resizeTimer;
  Timer? _statusTimer;

  VisualizerController get _controller => widget.controller;

  @override
  void initState() {
    super.initState();
    _scenes = context.read<VisualizerScenesProvider>();
    _controller.attach(resumesScene: _scenes.resumesScene);
    // The page darkens at once for the scene it waits for (not in the build).
    scheduleMicrotask(() {
      if (!_disposed) _scenes.sceneShowing = _controller.waiting.value;
    });
    WidgetsBinding.instance.addObserver(this);
    _syncVisible(WidgetsBinding.instance.lifecycleState);
    _statusTimer = Timer.periodic(
      const Duration(milliseconds: 250),
      (_) => _pollStatus(),
    );
  }

  // The accent colour last sent to Rust, for scenes; null before the first
  // build.
  int? _accent;
  // A problem shows in the corner until it is gone; scenes that start are
  // not announced.
  void _updateLabel(VisualizerStatusResponse status) {
    final error = status.lastError;
    _controller.problem.value = error;
    if (error != null) _controller.stopWaiting();
  }

  // Kept for dispose, where the context may no longer be used.
  late final VisualizerScenesProvider _scenes;

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
    _textureSubscription?.cancel();
    _controller.detach();
    // Not while the tree is being torn down: listeners rebuild.
    scheduleMicrotask(() => _scenes.sceneShowing = false);
    final controller = _controller;
    scheduleMicrotask(() => controller.textureId.value = null);

    // Rust unregisters its textures when the visualizer stops.
    if (_textureId != null || _starting) {
      StopVisualizerRequest().sendSignalToRust();
    }
    super.dispose();
  }

  Future<void> _start(Size logical, double pixelRatio) async {
    if (_starting || _textureId != null) return;
    _starting = true;

    try {
      final size = visualizerRenderSize(logical, pixelRatio);

      // Rust registers the texture once the visualizer runs and sends its id.
      _textureSubscription ??= VisualizerTextureResponse.rustSignalStream
          .listen((signal) => _showTexture(signal.message.textureId));

      // Tell Rust the order and the unchecked scenes before it picks the
      // first one.
      await _scenes.refresh();
      if (_disposed) return;
      StartVisualizerRequest(
        width: size.$1,
        height: size.$2,
        fps: _targetFps,
      ).sendSignalToRust();

      setState(() => _renderedSize = size);
      _publishAspect(size);
    } catch (e) {
      error$('Failed to start the visualizer: $e');
      // Leave `_starting` set so the next frames don't retry in a loop; showing
      // the surface again retries.
      _controller.problem.value = e.toString();
      _controller.stopWaiting();
    }
  }

  // Show the texture Rust sent, then tell Rust that it is on screen, so that a
  // texture it replaced can go.
  void _showTexture(int id) {
    if (_disposed || !mounted) return;
    setState(() => _textureId = id);
    _controller.textureId.value = id;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_disposed) return;
      VisualizerTextureShownRequest(textureId: id).sendSignalToRust();
    });
  }

  void _scheduleResize(Size logical, double pixelRatio) {
    if (_renderedSize == null) return;
    final size = visualizerRenderSize(logical, pixelRatio);

    // A pending resize to an older size must not fire, also when the size is
    // back to the rendered one.
    _resizeTimer?.cancel();
    if (size == _renderedSize) return;

    _resizeTimer = Timer(_resizeDebounce, () {
      if (_disposed) return;
      ResizeVisualizerRequest(
        width: size.$1,
        height: size.$2,
      ).sendSignalToRust();
      setState(() => _renderedSize = size);
      _publishAspect(size);
    });
  }

  // The shape of the picture, for a preview of the same texture.
  void _publishAspect((int, int) size) {
    if (size.$2 > 0) _controller.textureAspect.value = size.$1 / size.$2;
  }

  Future<void> _pollStatus() async {
    if (_renderedSize == null || _disposed) return;

    try {
      final signal = await awaitResponse(
        'VisualizerStatusRequest',
        VisualizerStatusResponse.rustSignalStream,
        timeout: const Duration(seconds: 1),
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) =>
            VisualizerStatusRequest(callId: callId).sendSignalToRust(),
      );
      if (_disposed || !mounted) return;
      final status = signal.message;

      _scenes.setInputs(status.inputs);
      _updateLabel(status);
      // No path yet: the scene the visualizer begins with is still being
      // built, and the page stays as it is until it shows.
      final path = status.running ? status.presetPath : '';
      _controller.scenePath.value = path.isEmpty ? null : path;
      _controller.sceneName.value = path.isEmpty ? null : status.presetName;
      if (path.isNotEmpty) {
        // The scene to resume next time, also after a restart.
        _scenes.setLastScene(path);
        _controller.stopWaiting();
      }
      // Scenes draw on black; the page darkens for them, and while it waits
      // for one.
      _scenes.sceneShowing = path.isEmpty
          ? _controller.waiting.value
          : path != kCoverWallScenePath;
    } on TimeoutException {
      // The next tick asks again.
    } on RequestFailure catch (failure) {
      error$('Could not read visualizer status: ${failure.detail}');
    }
  }

  @override
  Widget build(BuildContext context) {
    final pixelRatio = MediaQuery.devicePixelRatioOf(context);
    _syncAccent(FluentTheme.of(context).accentColor);

    return LayoutBuilder(
      builder: (context, constraints) {
        final size = constraints.biggest;
        if (size.width > 0 && size.height > 0 && size.isFinite) {
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (!mounted) return;
            if (_renderedSize == null) {
              _start(size, pixelRatio);
            } else {
              _scheduleResize(size, pixelRatio);
            }
          });
        }

        return _buildTexture();
      },
    );
  }

  /// Letterboxed: the texture keeps the aspect ratio of the size that was
  /// requested from Rust, so a window that changes shape never stretches it.
  /// Nothing is painted until there is a texture, so the cover wall behind
  /// shows while the visualizer starts, and stays if it cannot start at all.
  Widget _buildTexture() {
    final id = _textureId;
    final size = _renderedSize;
    if (id == null || size == null) return const SizedBox.expand();

    return ColoredBox(
      // Under the letterbox bars.
      color: Colors.black,
      child: Center(
        child: AspectRatio(
          aspectRatio: size.$1 / size.$2,
          child: Texture(textureId: id),
        ),
      ),
    );
  }
}
