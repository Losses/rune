import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:rune/utils/router/router_aware_flyout_controller.dart';
import 'package:rune/widgets/context_menu_wrapper.dart';

const _targetKey = Key('target');
final _target = find.byKey(_targetKey);

Future<List<Offset>> _pumpWrapper(WidgetTester tester) async {
  final opened = <Offset>[];
  await tester.pumpWidget(
    FluentApp(
      home: Center(
        child: ContextMenuWrapper(
          contextAttachKey: GlobalKey(),
          contextController: RouterAwareFlyoutController(),
          onContextMenu: opened.add,
          onMiddleClick: (_) {},
          child: const SizedBox(
            key: _targetKey,
            width: 100,
            height: 40,
            child: ColoredBox(color: Color(0xFF000000)),
          ),
        ),
      ),
    ),
  );
  return opened;
}

Future<void> _secondaryTap(WidgetTester tester) async {
  final gesture = await tester.startGesture(
    tester.getCenter(_target),
    kind: PointerDeviceKind.mouse,
    buttons: kSecondaryMouseButton,
  );
  await gesture.up();
  await tester.pump();
}

Future<void> _onPlatform(
  TargetPlatform platform,
  Future<void> Function() body,
) async {
  debugDefaultTargetPlatformOverride = platform;
  try {
    await body();
  } finally {
    debugDefaultTargetPlatformOverride = null;
  }
}

void main() {
  for (final platform in TargetPlatform.values) {
    testWidgets('secondary tap opens the menu on $platform', (tester) async {
      await _onPlatform(platform, () async {
        final opened = await _pumpWrapper(tester);
        await _secondaryTap(tester);

        expect(opened, hasLength(1));
      });
    });
  }

  for (final platform in [TargetPlatform.iOS, TargetPlatform.android]) {
    testWidgets('long press opens the menu on $platform', (tester) async {
      await _onPlatform(platform, () async {
        final opened = await _pumpWrapper(tester);
        await tester.longPress(_target);

        expect(opened, hasLength(1));
      });
    });
  }

  testWidgets('long press does not open the menu on desktop', (tester) async {
    await _onPlatform(TargetPlatform.macOS, () async {
      final opened = await _pumpWrapper(tester);
      await tester.longPress(_target);

      expect(opened, isEmpty);
    });
  });
}
