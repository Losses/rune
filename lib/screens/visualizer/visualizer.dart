import 'dart:async';
import 'dart:math';

import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../bindings/bindings.dart';
import '../../constants/configurations.dart';
import '../../utils/get_dir_path.dart';
import '../../utils/l10n.dart';
import '../../utils/rune_log.dart';
import '../../utils/settings_manager.dart';
import '../../utils/file_storage/mac_secure_manager.dart';
import '../../utils/navigation/utils/escape_from_visualizer.dart';
import '../../widgets/playback_controller/constants/playback_controller_height.dart';

const _channel = MethodChannel('not.ci.rune/visualizer');

const _targetFps = 60;
const _maxRenderWidth = 1920;
const _maxRenderHeight = 1080;
const _presetTransitionSeconds = 2.5;
const _controlsHideDelay = Duration(seconds: 3);
const _resizeDebounce = Duration(milliseconds: 300);

final _settings = SettingsManager();

/// The physical size of the view, shrunk to fit 1920x1080 with its aspect
/// ratio kept, rounded down to even numbers.
(int, int) _renderSize(Size logical, double pixelRatio) {
  var width = logical.width * pixelRatio;
  var height = logical.height * pixelRatio;
  final scale = min(1.0, min(_maxRenderWidth / width, _maxRenderHeight / height));
  width *= scale;
  height *= scale;
  return (max(2, width.floor() & ~1), max(2, height.floor() & ~1));
}

class VisualizerPage extends StatefulWidget {
  const VisualizerPage({super.key});

  @override
  State<VisualizerPage> createState() => _VisualizerPageState();
}

class _VisualizerPageState extends State<VisualizerPage> {
  final FocusNode _focusNode = FocusNode();

  int? _textureId;
  (int, int)? _renderedSize;
  bool _starting = false;
  // Why the texture could not be set up, shown instead of a silent black screen.
  String? _startError;
  bool _disposed = false;

  Timer? _resizeTimer;
  Timer? _hideTimer;
  Timer? _statusTimer;
  bool _controlsVisible = true;

  String? _presetDir;
  // The saved choice between the built-in pack and the folder; null = never chosen.
  VisualizerSource? _sourcePref;
  bool _firstPresetLoaded = false;
  VisualizerStatusResponse? _status;
  int _pollTick = 0;

  @override
  void initState() {
    super.initState();
    _scheduleHide();
    _statusTimer = Timer.periodic(
      const Duration(milliseconds: 250),
      (_) => _pollStatus(),
    );
  }

  @override
  void dispose() {
    _disposed = true;
    _resizeTimer?.cancel();
    _hideTimer?.cancel();
    _statusTimer?.cancel();
    _focusNode.dispose();

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
      // before Rust reads the saved folder.
      await MacSecureManager().completed;
      final dir = await _settings.getValue<String>(kVisualizerPresetDirKey);
      if (_disposed) {
        await _channel.invokeMethod<void>('dispose');
        return;
      }

      final savedSource = await _settings.getValue<String>(kVisualizerSourceKey);
      if (_disposed) {
        await _channel.invokeMethod<void>('dispose');
        return;
      }
      _sourcePref = switch (savedSource) {
        'builtin' => VisualizerSource.builtin,
        'folder' => VisualizerSource.folder,
        _ => null,
      };

      if (dir != null && dir.isNotEmpty) {
        SetVisualizerPresetDirRequest(path: dir).sendSignalToRust();
      }
      StartVisualizerRequest(
        width: size.$1,
        height: size.$2,
        fps: _targetFps,
      ).sendSignalToRust();

      setState(() {
        _textureId = id;
        _renderedSize = size;
        _presetDir = dir;
      });
    } catch (e) {
      error$('Failed to start the visualizer: $e');
      // Leave `_starting` set so the next frames don't retry in a loop; reopening retries.
      if (mounted) setState(() => _startError = e.toString());
    }
  }

  void _scheduleResize(Size logical, double pixelRatio) {
    if (_textureId == null) return;
    final size = _renderSize(logical, pixelRatio);
    if (size == _renderedSize) return;

    _resizeTimer?.cancel();
    _resizeTimer = Timer(_resizeDebounce, () {
      if (_disposed) return;
      ResizeVisualizerRequest(width: size.$1, height: size.$2)
          .sendSignalToRust();
      setState(() => _renderedSize = size);
    });
  }

  Future<void> _pollStatus() async {
    if (_textureId == null || _disposed) return;

    // About 1 Hz, but 4 Hz while waiting for the render thread to come up.
    final waiting = _status?.running != true;
    if (!waiting && ++_pollTick % 4 != 0) return;

    VisualizerStatusRequest().sendSignalToRust();
    try {
      final signal = await VisualizerStatusResponse.rustSignalStream.first
          .timeout(const Duration(seconds: 1));
      if (_disposed || !mounted) return;
      final status = signal.message;

      // Until the render thread is up there is nothing to send commands to.
      if (!_firstPresetLoaded && status.running && status.presetCount > 0) {
        // Rust starts with the folder if one is set, else with the built-in
        // pack. Apply a saved choice first and pick the preset on a later tick,
        // when the new source is in place.
        final wanted = _sourcePref;
        if (status.builtinAvailable &&
            wanted != null &&
            wanted != VisualizerSource.none &&
            wanted != status.source &&
            (wanted == VisualizerSource.builtin || _presetDir != null)) {
          SetVisualizerSourceRequest(source: wanted).sendSignalToRust();
        } else {
          _firstPresetLoaded = true;
          RandomVisualizerPresetRequest(transitionSeconds: 0)
              .sendSignalToRust();
        }
      }

      setState(() => _status = status);
    } on TimeoutException {
      // The next tick asks again.
    }
  }

  void _showControls() {
    if (!_controlsVisible) setState(() => _controlsVisible = true);
    _scheduleHide();
  }

  void _scheduleHide() {
    _hideTimer?.cancel();
    _hideTimer = Timer(_controlsHideDelay, () {
      if (mounted && _controlsVisible) {
        setState(() => _controlsVisible = false);
      }
    });
  }

  void _next() => NextVisualizerPresetRequest(
        transitionSeconds: _presetTransitionSeconds,
      ).sendSignalToRust();

  void _previous() => PreviousVisualizerPresetRequest(
        transitionSeconds: _presetTransitionSeconds,
      ).sendSignalToRust();

  void _random() => RandomVisualizerPresetRequest(
        transitionSeconds: _presetTransitionSeconds,
      ).sendSignalToRust();

  Future<void> _selectSource(VisualizerSource source) async {
    if (source == VisualizerSource.folder && _presetDir == null) {
      await _chooseFolder();
      return;
    }
    await _settings.setValue(
      kVisualizerSourceKey,
      source == VisualizerSource.builtin ? 'builtin' : 'folder',
    );
    if (_disposed) return;

    SetVisualizerSourceRequest(source: source).sendSignalToRust();
    RandomVisualizerPresetRequest(transitionSeconds: 0).sendSignalToRust();
    _firstPresetLoaded = true;
    setState(() => _sourcePref = source);
  }

  Future<void> _chooseFolder() async {
    // The system picker is a modal panel; keep the controls up meanwhile.
    _hideTimer?.cancel();
    final dir = await getDirPath();
    _scheduleHide();
    if (dir == null || _disposed) return;

    try {
      await MacSecureManager().saveBookmark(dir);
    } catch (e) {
      error$('Failed to keep access to $dir: $e');
    }
    await _settings.setValue(kVisualizerPresetDirKey, dir);
    await _settings.setValue(kVisualizerSourceKey, 'folder');
    if (_disposed) return;

    // Choosing a folder also switches to it.
    SetVisualizerPresetDirRequest(path: dir).sendSignalToRust();
    SetVisualizerSourceRequest(source: VisualizerSource.folder)
        .sendSignalToRust();
    RandomVisualizerPresetRequest(transitionSeconds: 0).sendSignalToRust();
    _firstPresetLoaded = true;
    setState(() {
      _presetDir = dir;
      _sourcePref = VisualizerSource.folder;
    });
  }

  @override
  Widget build(BuildContext context) {
    final pixelRatio = MediaQuery.devicePixelRatioOf(context);

    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.arrowRight): _next,
        const SingleActivator(LogicalKeyboardKey.arrowLeft): _previous,
        const SingleActivator(LogicalKeyboardKey.keyR): _random,
      },
      child: Focus(
        autofocus: true,
        focusNode: _focusNode,
        onKeyEvent: (_, event) {
          if (event is KeyDownEvent) _showControls();
          return KeyEventResult.ignored;
        },
        child: MouseRegion(
          onHover: (_) => _showControls(),
          cursor: _controlsVisible
              ? SystemMouseCursors.basic
              : SystemMouseCursors.none,
          child: ColoredBox(
            color: Colors.black,
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

                return Stack(
                  fit: StackFit.expand,
                  children: [
                    _buildTexture(),
                    _buildControls(context),
                  ],
                );
              },
            ),
          ),
        ),
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

  Widget _buildControls(BuildContext context) {
    final status = _status;
    final theme = FluentTheme.of(context);
    final error = status?.lastError;

    final hasPresets =
        _presetDir != null || (status?.builtinAvailable ?? false);
    final hint = _startError ??
        (!hasPresets
            ? S.of(context).visualizerNoPresets
            : error ?? status?.presetName ?? '');

    return IgnorePointer(
      ignoring: !_controlsVisible,
      child: AnimatedOpacity(
        opacity: _controlsVisible ? 1 : 0,
        duration: const Duration(milliseconds: 250),
        child: Stack(
          children: [
            Positioned(
              top: 16,
              left: 16,
              right: 16,
              child: Row(
                children: [
                  Tooltip(
                    message: S.of(context).visualizerExit,
                    child: IconButton(
                      icon: const Icon(Symbols.close, size: 20),
                      onPressed: escapeFromVisualizer,
                    ),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Text(
                      hint,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.typography.bodyStrong?.copyWith(
                        color: error != null && hasPresets
                            ? Colors.red
                            : Colors.white,
                      ),
                    ),
                  ),
                  if (status != null && status.running)
                    Text(
                      '${status.measuredFps.toStringAsFixed(0)} fps',
                      style: theme.typography.caption
                          ?.copyWith(color: Colors.white),
                    ),
                ],
              ),
            ),
            Positioned(
              left: 0,
              right: 0,
              bottom: playbackControllerHeight + 16,
              child: Row(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Tooltip(
                    message: S.of(context).visualizerPrevious,
                    child: IconButton(
                      icon: const Icon(Symbols.skip_previous, size: 24),
                      onPressed: _previous,
                    ),
                  ),
                  const SizedBox(width: 8),
                  Tooltip(
                    message: S.of(context).visualizerRandom,
                    child: IconButton(
                      icon: const Icon(Symbols.shuffle, size: 24),
                      onPressed: _random,
                    ),
                  ),
                  const SizedBox(width: 8),
                  Tooltip(
                    message: S.of(context).visualizerNext,
                    child: IconButton(
                      icon: const Icon(Symbols.skip_next, size: 24),
                      onPressed: _next,
                    ),
                  ),
                  const SizedBox(width: 24),
                  if (status != null && status.builtinAvailable) ...[
                    ToggleButton(
                      checked: status.source == VisualizerSource.builtin,
                      onChanged: (_) =>
                          _selectSource(VisualizerSource.builtin),
                      child: Text(S.of(context).visualizerSourceBuiltin),
                    ),
                    const SizedBox(width: 8),
                    ToggleButton(
                      checked: status.source == VisualizerSource.folder,
                      onChanged: (_) =>
                          _selectSource(VisualizerSource.folder),
                      child: Text(S.of(context).visualizerSourceFolder),
                    ),
                    const SizedBox(width: 8),
                  ],
                  Tooltip(
                    message: S.of(context).visualizerChooseFolder,
                    child: IconButton(
                      icon: const Icon(Symbols.folder_open, size: 24),
                      onPressed: _chooseFolder,
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
