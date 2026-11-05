import 'dart:async';
import 'dart:math' as math;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:provider/provider.dart';

import '../../../bindings/bindings.dart';
import '../../../providers/status.dart';
import '../../../providers/visualizer_lab.dart';
import '../../../providers/visualizer_scenes.dart';
import '../../../utils/api/seek.dart';
import '../../playback_controller/constants/playback_controller_height.dart';
import '../visualizer_controller.dart';

import 'lab_cards.dart';
import 'lab_data.dart';
import 'lab_equal_height_row.dart';
import 'lab_layout_mode.dart';
import 'lab_mode_builder.dart';
import 'lab_pages.dart';
import 'lab_pip.dart';
import 'lab_pip_layout.dart';
import 'lab_splitter.dart';
import 'lab_timeline.dart';

/// How long the overlay and the scrim take to fade out and in for a peek.
const labPeekDuration = Duration(milliseconds: 140);

// How tall the Scene card's picture asks to be when the card is free to
// choose; given a height it fills it.
const labScenePictureHeight = 160.0;

// The tallest the cards get on their own, as a part of the overlay's height.
const _cardsMaxHeightFraction = 0.55;

// The least the card row can be dragged to, and the least the timeline keeps.
const _cardsMinHeight = 96.0;
const _timelineMinHeight = 160.0;

// What the overlay has besides the cards and the timeline: its padding and
// the splitter.
const _overlayChromeHeight = 32.0;

// Under this height a card has a narrower margin.
const labCompactCardHeight = 150.0;

// The space between two cards, where the splitter between them is, as
// between the cards and the timeline.
const _cardGap = labSplitterThickness;

// The least the scene's picture can be dragged to, and the least the cards
// beside it keep of the row.
const _sceneMinWidth = 120.0;
const _restMinWidth = 240.0;

/// The lab's body on the visualizer page, under the playback controller that
/// has moved to the top of the window. It is only shown in the lab mode.
class VisualizerLabOverlay extends StatelessWidget {
  const VisualizerLabOverlay({super.key, required this.controller});

  /// The controller of the visualizer on the page, which has the texture of
  /// its picture.
  final VisualizerController controller;

  @override
  Widget build(BuildContext context) {
    final top =
        playbackControllerHeight + MediaQuery.of(context).viewPadding.top;

    return Positioned(
      top: top,
      left: 0,
      right: 0,
      bottom: 0,
      // It comes in after the controller has arrived, and goes out at once.
      child: LabModeBuilder(
        enterCurve: const Interval(0.35, 1, curve: labModeCurve),
        builder: (context, value, on, child) {
          if (value == 0) return const SizedBox.shrink();
          return IgnorePointer(
            ignoring: !on,
            child: Opacity(
              opacity: value,
              child: FractionalTranslation(
                translation: Offset(0, (1 - value) * 0.04),
                child: child,
              ),
            ),
          );
        },
        child: LabBody(controller: controller),
      ),
    );
  }
}

/// What holds a peek: the Alt key, or the picture of the scene.
enum _PeekSource { alt, preview }

/// The cards and the timeline. Holding Alt or the picture of the scene fades
/// it out, and the scrim behind it, until it is let go.
class LabBody extends StatefulWidget {
  const LabBody({super.key, required this.controller});

  final VisualizerController controller;

  @override
  State<LabBody> createState() => _LabBodyState();
}

class _LabBodyState extends State<LabBody> with WidgetsBindingObserver {
  final _held = <_PeekSource>{};
  late final VisualizerLabProvider _lab;
  final _cardsKey = GlobalKey();
  final _sceneKey = GlobalKey();

  @override
  void initState() {
    super.initState();
    _lab = context.read<VisualizerLabProvider>();
    WidgetsBinding.instance.addObserver(this);
    HardwareKeyboard.instance.addHandler(_onKey);
    // Not while the tree builds: listeners rebuild.
    final lab = _lab;
    scheduleMicrotask(() => lab.setBodyShown(true));
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    HardwareKeyboard.instance.removeHandler(_onKey);
    // Not while the tree is being torn down: listeners rebuild.
    final lab = _lab;
    scheduleMicrotask(() {
      lab.setBodyShown(false);
      lab.setPeeking(false);
    });
    super.dispose();
  }

  bool _onKey(KeyEvent event) {
    final key = event.logicalKey;
    if (key == LogicalKeyboardKey.altLeft ||
        key == LogicalKeyboardKey.altRight) {
      if (event is KeyDownEvent) {
        _hold(_PeekSource.alt, true);
      } else if (event is KeyUpEvent &&
          !HardwareKeyboard.instance.isAltPressed) {
        _hold(_PeekSource.alt, false);
      }
    }
    // Other widgets keep the key.
    return false;
  }

  // A window that loses the focus never sees the key come up.
  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state != AppLifecycleState.resumed) {
      _held.clear();
      _lab.setPeeking(false);
    }
  }

  void _hold(_PeekSource source, bool on) {
    if (on ? _held.add(source) : _held.remove(source)) {
      _lab.setPeeking(_held.isNotEmpty);
    }
  }

  // ---------- seeking ----------

  void _seek(double seconds) =>
      SeekRequest(positionSeconds: seconds).sendSignalToRust();

  // While a drag moves, the song follows at a steady pace.
  void _scrub(double seconds) {
    final status = context.read<PlaybackStatusProvider>().playbackStatus;
    if (status.duration > 0) {
      seek(seconds / status.duration * 100, status);
    }
  }

  // ---------- build ----------

  // Cached so that the theme under the overlay keeps its identity.
  FluentThemeData? _dark;
  FluentThemeData? _darkFor;

  FluentThemeData _darkTheme(FluentThemeData theme) {
    if (!identical(_darkFor, theme)) {
      _darkFor = theme;
      _dark = FluentThemeData(
        brightness: Brightness.dark,
        accentColor: theme.accentColor,
        visualDensity: theme.visualDensity,
        fontFamily: theme.typography.body?.fontFamily,
      );
    }
    return _dark!;
  }

  @override
  Widget build(BuildContext context) {
    final peeking = context.select<VisualizerLabProvider, bool>(
      (p) => p.peeking,
    );
    final cardsHeight = context.select<VisualizerLabProvider, double?>(
      (p) => p.cardsHeight,
    );
    final sceneWidth = context.select<VisualizerLabProvider, double?>(
      (p) => p.sceneWidth,
    );
    final pip = context.select<VisualizerLabProvider, LabPipLayout>(
      (p) => p.pipLayout,
    );
    final theme = FluentTheme.of(context);

    return FluentTheme(
      data: _darkTheme(theme),
      // The overlay ignores new pointers while it is faded out; a press that
      // began before keeps its pointer, and gets its pointer up.
      child: IgnorePointer(
        ignoring: peeking,
        child: AnimatedOpacity(
          opacity: peeking ? 0 : 1,
          duration: labPeekDuration,
          // The overlay takes the pointer, so a click on it does not fall
          // through to the page behind.
          child: Listener(
            behavior: HitTestBehavior.opaque,
            child: LayoutBuilder(
              builder: (context, constraints) {
                final narrow = constraints.maxWidth < 720;
                final padding = narrow
                    ? const EdgeInsets.fromLTRB(8, 8, 8, 10)
                    : const EdgeInsets.fromLTRB(14, 10, 14, 12);
                final layout = labLayoutOf(context);
                if (layout.paged) {
                  return _pages(layout, constraints, padding, pip);
                }
                return Padding(
                  padding: padding,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      _cards(
                        peeking,
                        constraints.maxHeight,
                        cardsHeight,
                        sceneWidth,
                      ),
                      LabSplitter(
                        currentSize: () =>
                            _cardsKey.currentContext?.size?.height ??
                            _cardsMinHeight,
                        minSize: _cardsMinHeight,
                        maxSize: _cardsMaxHeight(constraints.maxHeight),
                        onSize: (height, {required save}) =>
                            _lab.setCardsHeight(height, save: save),
                      ),
                      Expanded(child: _timeline()),
                    ],
                  ),
                );
              },
            ),
          ),
        ),
      ),
    );
  }

  // The cards and the timeline as pages of their own, with the picture of the
  // scene floating over them.
  Widget _pages(
    LabLayout layout,
    BoxConstraints constraints,
    EdgeInsets padding,
    LabPipLayout pip,
  ) {
    final view = MediaQuery.viewPaddingOf(context);
    // The screen's own edges (a notch, a home bar) are kept clear; the top is
    // already under the controller.
    final pages = padding.copyWith(
      left: padding.left + view.left,
      right: padding.right + view.right,
      bottom: padding.bottom + view.bottom,
    );
    return Stack(
      children: [
        Positioned.fill(
          child: LabPages(
            axis: layout == LabLayout.pagesAcross
                ? Axis.horizontal
                : Axis.vertical,
            padding: pages,
            cards: _playCard(fill: true),
            timeline: _timeline(),
          ),
        ),
        LabPip(
          controller: widget.controller,
          area: constraints.biggest,
          insets: pages,
          layout: pip,
          onLayout: _lab.setPipLayout,
          onHold: (on) => _hold(_PeekSource.preview, on),
        ),
      ],
    );
  }

  Widget _playCard({bool fill = false}) => LabCard(
    label: 'Play',
    hang: labTitleInset,
    fill: fill,
    child: LabPlayBody(
      lab: _lab,
      header: _ScenePicker(controller: widget.controller),
    ),
  );

  // The most a dragged card row can be, so that the timeline keeps its share.
  double _cardsMaxHeight(double overlayHeight) => math.max(
    _cardsMinHeight,
    overlayHeight - _timelineMinHeight - _overlayChromeHeight,
  );

  Widget _cards(
    bool peeking,
    double overlayHeight,
    double? height,
    double? sceneWidth,
  ) {
    // Left to themselves the cards are as tall as the tallest content, up to
    // this; past it they flow into more columns.
    final maxHeight = overlayHeight * _cardsMaxHeightFraction;
    // A height from the splitter is held to the room the window has now.
    final fixed = height?.clamp(
      _cardsMinHeight,
      _cardsMaxHeight(overlayHeight),
    );
    // The row fills the overlay's width and goes no further: the scene's
    // picture as wide as the splitter between the cards sets it (or as its
    // shape makes it), leaving the card beside it its least, and that card
    // the rest. They share one height.
    return LayoutBuilder(
      builder: (context, constraints) {
        final width = constraints.maxWidth;
        final sceneMax = math.max(
          _sceneMinWidth,
          width - _cardGap - _restMinWidth,
        );
        return LabEqualHeightRow(
          key: _cardsKey,
          gap: 0,
          maxHeight: maxHeight,
          height: fixed,
          fill: true,
          children: [
            ConstrainedBox(
              constraints: BoxConstraints(maxWidth: sceneMax),
              child: LabCard(
                key: _sceneKey,
                label: 'Scene',
                framed: false,
                fill: true,
                child: LabScenePreview(
                  controller: widget.controller,
                  width: sceneWidth?.clamp(_sceneMinWidth, sceneMax),
                  peeking: peeking,
                  onHold: (on) => _hold(_PeekSource.preview, on),
                ),
              ),
            ),
            LabSplitter(
              axis: Axis.horizontal,
              currentSize: () =>
                  _sceneKey.currentContext?.size?.width ?? _sceneMinWidth,
              minSize: _sceneMinWidth,
              maxSize: sceneMax,
              onSize: (width, {required save}) =>
                  _lab.setSceneWidth(width, save: save),
            ),
            _playCard(),
          ],
        );
      },
    );
  }

  Widget _timeline() {
    return ValueListenableBuilder<LabTimelineData>(
      valueListenable: _lab.timeline,
      builder: (context, data, _) {
        return LabTimeline(
          data: data,
          rows: _lab.rowsModel,
          onSeek: _seek,
          onScrub: _scrub,
          labelWidth: context.select<VisualizerLabProvider, double?>(
            (p) => p.labelWidth,
          ),
          onLabelWidth: (width, {required save}) =>
              _lab.setLabelWidth(width, save: save),
        );
      },
    );
  }
}

/// A card above the timeline. It is as wide as its content, or as the row
/// leaves it, and as tall as the row gives it. It has no title: what it shows
/// says what it is.
class LabCard extends StatelessWidget {
  const LabCard({
    super.key,
    required this.label,
    this.framed = true,
    this.fill = false,
    this.hang = 0,
    required this.child,
  });

  /// What the card is, for a screen reader.
  final String label;

  /// Whether the body sits on a bordered panel, rather than alone.
  final bool framed;

  /// Whether the body takes all the height when the card has a height of its
  /// own, rather than as much as it needs.
  final bool fill;

  /// How much less the margin is at the top and left, for a body whose first
  /// line reaches into it (see [labTitleInset]).
  final double hang;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Semantics(
      container: true,
      label: label,
      child: LayoutBuilder(
        builder: (context, constraints) {
          // The first pass of the row is loose; the body asks for what it
          // needs. Given a height it fills it. The widget stays the same
          // between the passes, so the picture is not rebuilt.
          final body = Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Flexible(
                key: const ValueKey('body'),
                fit: fill && constraints.hasTightHeight
                    ? FlexFit.tight
                    : FlexFit.loose,
                child: child,
              ),
            ],
          );
          if (!framed) return body;
          final compact = constraints.maxHeight < labCompactCardHeight;
          final margin = compact ? 10.0 : 16.0;
          return Card(
            padding: EdgeInsets.fromLTRB(
              margin - hang,
              margin - hang,
              margin,
              margin,
            ),
            // A body wider than the card, past the narrowest its lines get,
            // stays in it.
            child: ClipRect(child: body),
          );
        },
      ),
    );
  }
}

/// The scenes the app lists and the one on screen, for the Play card to
/// switch between; picking one shows it.
class _ScenePicker extends StatelessWidget {
  const _ScenePicker({required this.controller});

  final VisualizerController controller;

  @override
  Widget build(BuildContext context) {
    final scenes = context
        .select<VisualizerScenesProvider, List<VisualizerScene>>(
          (p) => p.scenes,
        );
    return ListenableBuilder(
      listenable: Listenable.merge([
        controller.scenePath,
        controller.sceneName,
      ]),
      builder: (context, _) => LabScenePicker(
        scenes: scenes,
        current: controller.scenePath.value,
        name: controller.sceneName.value,
        onPicked: controller.loadPreset,
      ),
    );
  }
}

/// The picture of the visualizer, from the texture the full-size surface
/// shows. It is as tall as the card gives it, up to [labScenePictureHeight]
/// when free, and as wide as the texture's shape makes it, or [width] wide
/// with the picture fitted in it on black. Holding it peeks.
class LabScenePreview extends StatelessWidget {
  const LabScenePreview({
    super.key,
    required this.controller,
    this.width,
    required this.peeking,
    this.onHold,
  });

  final VisualizerController controller;

  /// How wide it is, the picture whole in it and black around it; none for
  /// as wide as the picture's shape makes it at its height.
  final double? width;
  final bool peeking;

  /// Called with true when it is pressed and false when it is let go. Without
  /// it the picture takes no press of its own, for a parent that does.
  final ValueChanged<bool>? onHold;

  @override
  Widget build(BuildContext context) {
    final hold = onHold;
    final picture = ClipRRect(
      borderRadius: BorderRadius.circular(4),
      child: LayoutBuilder(
        builder: (context, constraints) {
          final height = math.max(
            constraints.minHeight,
            math.min(labScenePictureHeight, constraints.maxHeight),
          );
          return ListenableBuilder(
            listenable: Listenable.merge([
              controller.textureId,
              controller.textureAspect,
            ]),
            builder: (context, _) {
              final id = controller.textureId.value;
              final aspect = controller.textureAspect.value;
              final shape = aspect > 0 ? aspect : 16 / 9;
              return SizedBox(
                width: width ?? height * shape,
                height: height,
                child: ColoredBox(
                  color: Colors.black,
                  child: id == null
                      ? const SizedBox.expand()
                      : Center(
                          child: AspectRatio(
                            aspectRatio: shape,
                            child: Texture(textureId: id),
                          ),
                        ),
                ),
              );
            },
          );
        },
      ),
    );
    if (hold == null) return picture;
    return MouseRegion(
      cursor: peeking ? SystemMouseCursors.zoomOut : SystemMouseCursors.zoomIn,
      child: Listener(
        behavior: HitTestBehavior.opaque,
        onPointerDown: (e) {
          if (e.buttons & kPrimaryButton != 0) hold(true);
        },
        onPointerUp: (_) => hold(false),
        onPointerCancel: (_) => hold(false),
        child: picture,
      ),
    );
  }
}
