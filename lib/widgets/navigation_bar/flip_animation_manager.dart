import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/logger.dart';
import '../router/router_animation.dart';

import './flip_text.dart';
import './flip_text_animation.dart';
import './utils/text_style_sheet.dart';

class FlipAnimationManager extends StatefulWidget {
  final Widget child;

  const FlipAnimationManager({super.key, required this.child});

  static FlipAnimationManagerState? of(BuildContext context) {
    return context.findAncestorStateOfType<FlipAnimationManagerState>();
  }

  @override
  FlipAnimationManagerState createState() => FlipAnimationManagerState();
}

class FlipAnimationManagerState extends State<FlipAnimationManager> {
  // The texts that hold each key, the newest last, which is the one that
  // flies. A page opened over another with the same key holds it while it is
  // open, and the page under it holds it again once it closes.
  final Map<String, List<GlobalKey>> _holders = {};
  final Map<String, FlipTextPositions> _cachedPositions = {};
  final Map<String, FlipTextStyles> _cachedStyles = {};
  final List<OverlayEntry> _overlayEntries = [];
  VoidCallback? _cancelAnimation;
  bool _disposing = false;
  (Size, double, EdgeInsets, EdgeInsets, EdgeInsets)? _viewportMetrics;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final media = MediaQuery.of(context);
    final metrics = (
      media.size,
      media.devicePixelRatio,
      media.padding,
      media.viewPadding,
      media.viewInsets,
    );
    if (_viewportMetrics != null && metrics != _viewportMetrics) {
      _cachedPositions.clear();
      final animation = _cancelAnimation;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted &&
            animation != null &&
            identical(animation, _cancelAnimation)) {
          _stopAllAnimations();
        }
      });
    }
    _viewportMetrics = metrics;
  }

  @override
  void dispose() {
    _disposing = true;
    _stopAllAnimations();
    super.dispose();
  }

  void registerStyle(
    String key,
    double scale,
    double fontWeight,
    Color color,
    double alpha,
  ) {
    final style = _cachedStyles[key];

    if (style == null) {
      _cachedStyles[key] = FlipTextStyles(
        scale: scale,
        fontWeight: fontWeight,
        color: color,
        alpha: alpha,
      );
    } else {
      style.scale = scale;
      style.fontWeight = fontWeight;
      style.color = color;
      style.alpha = alpha;
    }
  }

  GlobalKey? _holder(String key) => _holders[key]?.lastOrNull;

  void registerKey(String key, GlobalKey globalKey) {
    final holders = _holders.putIfAbsent(key, () => []);
    holders.remove(globalKey);
    holders.add(globalKey);

    WidgetsBinding.instance.addPostFrameCallback((_) {
      cachePositionWithKey(key);
    });
  }

  /// Forgets the text [globalKey] under [key]. When it was the one that flies,
  /// where it was is forgotten too, so that no flight starts or ends there.
  void unregisterKey(String key, GlobalKey globalKey) {
    final holders = _holders[key];
    if (holders == null) return;
    final held = holders.lastOrNull == globalKey;
    holders.remove(globalKey);
    if (holders.isEmpty) _holders.remove(key);
    if (held) _cachedPositions.remove(key);
  }

  void cachePositionWithKey(String key) {
    final globalKey = _holder(key);
    if (globalKey != null) {
      final styles = getPosition(key, globalKey);

      if (styles != null) {
        _cachedPositions[key] = styles;

        // logger.i("Cached bounding box cached: $key");
      } else {
        // logger.w("Cached bounding not found: $key");
      }
    } else {
      // logger.w("Key not registered: $key");
    }
  }

  void _stopAllAnimations() {
    _cancelAnimation?.call();
    _cancelAnimation = null;
    for (var overlayEntry in _overlayEntries) {
      overlayEntry.remove();
    }
    _overlayEntries.clear();
  }

  void _setVisibility(GlobalKey holder, bool visible) {
    final context = holder.currentContext;
    if (context != null && context.mounted) {
      final state = context.findAncestorStateOfType<FlipTextState>();
      state?.setVisibility(visible);
    }
  }

  Future<bool> flipAnimation(String fromKey, String toKey) async {
    _stopAllAnimations(); // Stop all ongoing animations

    // Where the texts are as the page changes: a text that leaves with the
    // old page is gone, and its place forgotten, after the frame.
    cachePositionWithKey(fromKey);
    cachePositionWithKey(toKey);
    final fromBefore = _cachedPositions[fromKey];
    final toBefore = _cachedPositions[toKey];

    // The texts hidden while the copy flies, to show again after it.
    final hidden = <GlobalKey>{};
    void hide(String key) {
      final holder = _holder(key);
      if (holder == null) return;
      hidden.add(holder);
      _setVisibility(holder, false);
    }

    void showHidden() {
      for (final holder in hidden) {
        _setVisibility(holder, true);
      }
      hidden.clear();
    }

    hide(fromKey);
    hide(toKey);

    final completer = Completer<bool>();
    var cancelled = false;
    _cancelAnimation = () {
      cancelled = true;
      if (!_disposing) showHidden();
      if (!completer.isCompleted) completer.complete(false);
    };

    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || cancelled) return;
      cachePositionWithKey(fromKey);
      cachePositionWithKey(toKey);

      final fromPosition = _cachedPositions[fromKey] ?? fromBefore;
      final toPosition = _cachedPositions[toKey] ?? toBefore;
      final fromStyles = _cachedStyles[fromKey];
      final toStyles = _cachedStyles[toKey];

      // The texts that hold the keys now are the ones that fly. One hidden
      // before the page changed that holds neither any more shows again.
      final holders = {_holder(fromKey), _holder(toKey)};
      for (final holder in hidden.where((x) => !holders.contains(x))) {
        _setVisibility(holder, true);
      }
      hidden.retainWhere(holders.contains);
      hide(fromKey);
      hide(toKey);

      if (fromPosition == null ||
          toPosition == null ||
          fromStyles == null ||
          toStyles == null ||
          (!fromPosition.context.mounted && !toPosition.context.mounted)) {
        _stopAllAnimations();
        return;
      }

      final mountedContext = toPosition.context.mounted
          ? toPosition.context
          : fromPosition.context;
      final transformWidget = mountedContext.widget as Transform?;
      final textWidget = transformWidget?.child as Text?;

      // Declare the overlayEntry variable first
      late OverlayEntry overlayEntry;

      // Create a text overlay in the animation layer and perform a smooth transition animation
      overlayEntry = OverlayEntry(
        builder: (context) => FlipTextAnimation(
          fromPositions: fromPosition,
          toPositions: toPosition,
          fromStyles: fromStyles,
          toStyles: toStyles,
          text: textWidget?.data ?? '',
          onAnimationComplete: () {
            overlayEntry.remove();
            _overlayEntries.remove(overlayEntry);
            // Show the texts again, those still mounted.
            showHidden();
            _cancelAnimation = null;
            if (!completer.isCompleted) completer.complete(true);
          },
        ),
      );

      _overlayEntries.add(overlayEntry);

      Overlay.of(context, rootOverlay: true).insert(overlayEntry);
    });

    return completer.future;
  }

  double? getFontVariationValue(List<FontVariation>? variations, String name) {
    if (variations == null) return null;

    for (var variation in variations) {
      if (variation.axis == name) {
        return variation.value;
      }
    }
    return null;
  }

  FlipTextPositions? getPosition(String key, GlobalKey globalKey) {
    final context = globalKey.currentContext;

    if (context == null) {
      logger.w("Context not found: $key");
      return null;
    }

    if (!context.mounted) {
      logger.w("Widget not mounted: $key");
      return null;
    }

    final box = context.findRenderObject() as RenderBox;
    final overlay = Overlay.of(this.context, rootOverlay: true);
    final overlayBox = overlay.context.findRenderObject() as RenderBox;
    // A text on a page is where it rests, not where the page's transition
    // paints it for now: the copy flies there and the text shows there.
    final position =
        restingPositionInPage(context, overlayBox) ??
        box.localToGlobal(Offset.zero, ancestor: overlayBox);

    return FlipTextPositions(context: context, position: position);
  }

  @override
  Widget build(BuildContext context) {
    return widget.child;
  }
}
