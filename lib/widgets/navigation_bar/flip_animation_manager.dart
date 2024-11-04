import 'dart:async';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/logger.dart';

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
  final Map<String, GlobalKey> _registeredKeys = {};
  final Map<String, FlipTextPositions> _cachedPositions = {};
  final Map<String, FlipTextStyles> _cachedStyles = {};
  final List<OverlayEntry> _overlayEntries = [];

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

  void registerKey(String key, GlobalKey globalKey) {
    _registeredKeys[key] = globalKey;

    WidgetsBinding.instance.addPostFrameCallback((_) {
      cachePositionWithKey(key);
    });
  }

  void unregisterKey(String key) {
    _registeredKeys.remove(key);
  }

  void cachePositionWithKey(String key) {
    if (_registeredKeys.containsKey(key)) {
      final globalKey = _registeredKeys[key];

      final styles = getPosition(key, globalKey!);

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
    for (var overlayEntry in _overlayEntries) {
      overlayEntry.remove();
    }
    _overlayEntries.clear();
  }

  void _setVisibility(String key, bool visible) {
    if (_registeredKeys.containsKey(key)) {
      final globalKey = _registeredKeys[key];
      final context = globalKey?.currentContext;
      if (context != null && context.mounted) {
        final state = context.findAncestorStateOfType<FlipTextState>();
        state?.setVisibility(visible);
      }
    }
  }

  Future<bool> flipAnimation(String fromKey, String toKey) async {
    _stopAllAnimations(); // Stop all ongoing animations

    cachePositionWithKey(fromKey);
    cachePositionWithKey(toKey);

    _setVisibility(fromKey, false);
    _setVisibility(toKey, false);

    final completer = Completer<bool>();

    WidgetsBinding.instance.addPostFrameCallback((_) {
      cachePositionWithKey(fromKey);
      cachePositionWithKey(toKey);

      final fromPosition = _cachedPositions[fromKey];
      final toPosition = _cachedPositions[toKey];
      final fromStyles = _cachedStyles[fromKey];
      final toStyles = _cachedStyles[toKey];

      if (fromPosition == null && toPosition != null) {
        _setVisibility(toKey, false);
      } else if (fromPosition != null && toPosition == null) {
        _setVisibility(fromKey, false);
      } else {
        // Hide elements before starting animation
        _setVisibility(fromKey, false);
        _setVisibility(toKey, false);
      }

      if (fromPosition == null) {
        completer.complete(false);
        return;
      }
      if (toPosition == null) {
        completer.complete(false);
        return;
      }

      if (fromStyles == null) {
        completer.complete(false);
        return;
      }
      if (toStyles == null) {
        completer.complete(false);
        return;
      }

      if (!fromPosition.context.mounted && !toPosition.context.mounted) {
        completer.complete(false);
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
            // Show elements after animation ends if they are still mounted
            if (fromPosition.context.mounted) {
              _setVisibility(fromKey, true);
            }
            if (toPosition.context.mounted) {
              _setVisibility(toKey, true);
            }
            completer.complete(true);
          },
        ),
      );

      _overlayEntries.add(overlayEntry);

      Overlay.of(context, rootOverlay: true).insert(overlayEntry);

      (overlayEntry.builder(context) as FlipTextAnimation).createState();
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
    final position = box.localToGlobal(Offset.zero);

    return FlipTextPositions(
      context: context,
      position: position,
    );
  }

  @override
  Widget build(BuildContext context) {
    return widget.child;
  }
}
