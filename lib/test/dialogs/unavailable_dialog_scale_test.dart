import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/utils/dialogs/unavailable_dialog_on_band.dart';
import 'package:rune/widgets/band_screen_fallback_button.dart';

class _Responsive extends ChangeNotifier implements ResponsiveProvider {
  _Responsive(this.device);
  DeviceType device;
  void change(DeviceType value) {
    device = value;
    notifyListeners();
  }

  @override
  DeviceType getActiveDeviceType(List<DeviceType> deviceTypes) => device;
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  testWidgets(
    'open form retains editing state through tiny viewport and back',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(800, 600);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetPhysicalSize);
      final responsive = _Responsive(DeviceType.tv);
      final controller = TextEditingController();
      final focus = FocusNode();
      await tester.pumpWidget(
        ChangeNotifierProvider<ResponsiveProvider>.value(
          value: responsive,
          child: FluentApp(
            builder: (context, child) => MediaQuery(
              data: MediaQuery.of(
                context,
              ).copyWith(textScaler: const TextScaler.linear(1.5)),
              child: child!,
            ),
            home: UnavailableDialogOnBand(
              $close: (_) {},
              child: ContentDialog(
                title: const Text('Create playlist'),
                content: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const SizedBox(height: 16),
                    InfoLabel(
                      label: 'Title',
                      child: TextBox(controller: controller, focusNode: focus),
                    ),
                    const SizedBox(height: 16),
                    InfoLabel(
                      label: 'Group',
                      child: AutoSuggestBox<String>(items: []),
                    ),
                    const SizedBox(height: 8),
                  ],
                ),
                actions: [
                  Button(onPressed: () {}, child: const Text('Cancel')),
                ],
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.byType(TextBox).first);
      await tester.enterText(find.byType(TextBox).first, 'draft');
      controller.value = controller.value.copyWith(
        composing: const TextRange(start: 0, end: 5),
      );
      await tester.pump();
      final input = find.descendant(
        of: find.byType(TextBox).first,
        matching: find.byType(EditableText),
      );
      final original = tester.state(input);
      expect(focus.hasFocus, isTrue);
      tester.view.physicalSize = const Size(180, 120);
      responsive.change(DeviceType.dock);
      await tester.pump();
      expect(
        tester.state(
          find.descendant(
            of: find.byType(TextBox).first,
            matching: find.byType(EditableText),
          ),
        ),
        same(original),
      );
      expect(controller.text, 'draft');
      expect(controller.value.composing, const TextRange(start: 0, end: 5));
      expect(focus.hasFocus, isTrue);
      expect(find.byType(BandScreenFallbackButton), findsNothing);
      expect(tester.takeException(), isNull);
      tester.view.physicalSize = const Size(800, 600);
      responsive.change(DeviceType.tv);
      await tester.pump();
      expect(
        tester.state(
          find.descendant(
            of: find.byType(TextBox).first,
            matching: find.byType(EditableText),
          ),
        ),
        same(original),
      );
      expect(focus.hasFocus, isTrue);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      controller.dispose();
      focus.dispose();
      responsive.dispose();
    },
  );

  testWidgets('dialog first opened compact shows existing fallback', (
    tester,
  ) async {
    final responsive = _Responsive(DeviceType.dock);
    await tester.pumpWidget(
      ChangeNotifierProvider<ResponsiveProvider>.value(
        value: responsive,
        child: FluentApp(
          builder: (context, child) => MediaQuery(
            data: MediaQuery.of(
              context,
            ).copyWith(textScaler: const TextScaler.linear(1.5)),
            child: child!,
          ),
          home: UnavailableDialogOnBand($close: (_) {}, child: const TextBox()),
        ),
      ),
    );
    expect(find.byType(BandScreenFallbackButton), findsOneWidget);
    expect(find.byType(TextBox), findsNothing);
    await tester.pumpWidget(const SizedBox());
    responsive.dispose();
  });
}
