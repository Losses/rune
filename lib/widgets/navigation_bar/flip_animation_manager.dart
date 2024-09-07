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
  final Map<String, TextStyleSheet> _cachedBoundingBox = {};
  final List<OverlayEntry> _overlayEntries = [];

  void registerKey(String key, GlobalKey globalKey) {
    _registeredKeys[key] = globalKey;

    WidgetsBinding.instance.addPostFrameCallback((_) {
      cacheStyleSheetWithKey(key);
    });
  }

  void unregisterKey(String key) {
    _registeredKeys.remove(key);
  }

  void cacheStyleSheetWithKey(String key) {
    if (_registeredKeys.containsKey(key)) {
      final globalKey = _registeredKeys[key];

      final boundingBox = getStyleSheet(key, globalKey!);

      if (boundingBox != null) {
        _cachedBoundingBox[key] = boundingBox;

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

    cacheStyleSheetWithKey(fromKey);
    cacheStyleSheetWithKey(toKey);

    _setVisibility(fromKey, false);
    _setVisibility(toKey, false);

    final completer = Completer<bool>();

    WidgetsBinding.instance.addPostFrameCallback((_) {
      cacheStyleSheetWithKey(fromKey);
      cacheStyleSheetWithKey(toKey);

      final fromBoundingBox = _cachedBoundingBox[fromKey];
      final toBoundingBox = _cachedBoundingBox[toKey];

      // Hide elements before starting animation
      _setVisibility(fromKey, false);
      _setVisibility(toKey, false);

      if (fromBoundingBox == null) {
        completer.complete(false);
        return;
      }
      if (toBoundingBox == null) {
        completer.complete(false);
        return;
      }

      if (!fromBoundingBox.context.mounted && !toBoundingBox.context.mounted) {
        completer.complete(false);
        return;
      }

      final mountedContext = toBoundingBox.context.mounted
          ? toBoundingBox.context
          : fromBoundingBox.context;
      final transformWidget = mountedContext.widget as Transform?;
      final textWidget = transformWidget?.child as Text?;

      // Declare the overlayEntry variable first
      late OverlayEntry overlayEntry;

      // Create a text overlay in the animation layer and perform a smooth transition animation
      overlayEntry = OverlayEntry(
        builder: (context) => FlipTextAnimation(
          fromStyles: fromBoundingBox,
          toStyles: toBoundingBox,
          text: textWidget?.data ?? '',
          onAnimationComplete: () {
            overlayEntry.remove();
            _overlayEntries.remove(overlayEntry);
            // Show elements after animation ends if they are still mounted
            if (fromBoundingBox.context.mounted) {
              _setVisibility(fromKey, true);
            }
            if (toBoundingBox.context.mounted) {
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

  TextStyleSheet? getStyleSheet(String key, GlobalKey globalKey) {
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

    final transformWidget = context.widget as Transform;
    final textWidget = transformWidget.child as Text;
    final style = textWidget.style;

    final DefaultTextStyle defaultTextStyle = DefaultTextStyle.of(context);
    TextStyle? effectiveTextStyle = style;
    if (style == null || style.inherit) {
      effectiveTextStyle = defaultTextStyle.style.merge(style);
    }
    if (MediaQuery.boldTextOf(context)) {
      effectiveTextStyle = effectiveTextStyle!
          .merge(const TextStyle(fontWeight: FontWeight.bold));
    }

    return TextStyleSheet(
        context: context,
        position: position,
        scale: transformWidget.transform.row0[0],
        fontWeight:
            getFontVariationValue(effectiveTextStyle?.fontVariations, 'wght') ??
                400,
        color: (effectiveTextStyle?.color)!);
  }

  @override
  Widget build(BuildContext context) {
    return widget.child;
  }
}
