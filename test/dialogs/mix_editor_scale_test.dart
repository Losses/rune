import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/utils/dialogs/mix/widgets/mix_editor.dart';
import 'package:rune/utils/dialogs/mix/widgets/mix_editor_controller.dart';
import 'package:rune/utils/dialogs/mix/widgets/input_section.dart';

class _Responsive extends ChangeNotifier implements ResponsiveProvider {
  @override
  bool smallerOrEqualTo(DeviceType type, [bool autoOrientation = true]) =>
      false;
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  testWidgets(
    'editor input and externally owned controller survive a layout change',
    (tester) async {
      final controller = MixEditorController();
      final key = GlobalKey();
      Widget app(bool compact, [MixEditorController? supplied]) {
        final editor = MixEditor(
          key: key,
          controller: supplied ?? controller,
          initialSuggestions: (_) async => [],
          groupSuggestions: () async => [],
        );
        return ChangeNotifierProvider<ResponsiveProvider>(
          create: (_) => _Responsive(),
          child: FluentApp(
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            home: ScaffoldPage(
              content: MediaQuery(
                data: const MediaQueryData(textScaler: TextScaler.linear(5)),
                child: Align(
                  alignment: Alignment.topLeft,
                  child: SizedBox(
                    height: 100,
                    child: compact
                        ? editor
                        : Row(children: [SizedBox(width: 380, child: editor)]),
                  ),
                ),
              ),
            ),
          ),
        );
      }

      await tester.pumpWidget(app(false));
      final input = find.byType(TextBox).first;
      await tester.enterText(input, 'draft');
      await tester.pump();
      final state = tester.state(find.byType(MixEditor));
      final editable = tester.state(
        find.descendant(of: input, matching: find.byType(EditableText)),
      );
      await tester.pumpWidget(app(true));
      await tester.pump();
      expect(tester.state(find.byType(MixEditor)), same(state));
      expect(
        tester.state(
          find.descendant(
            of: find.byType(TextBox).first,
            matching: find.byType(EditableText),
          ),
        ),
        same(editable),
      );
      expect(controller.titleController.text, 'draft');
      expect(editable, isA<EditableTextState>());
      final replacement = MixEditorController();
      await tester.pumpWidget(app(true, replacement));
      await tester.pump();
      await tester.enterText(find.byType(TextBox).first, 'replacement');
      expect(replacement.titleController.text, 'replacement');
      expect(controller.titleController.text, 'draft');
      await tester.pumpWidget(const SizedBox());
      replacement.titleController.text = 'still external';
      replacement.dispose();
      controller.titleController.text = 'still owned by parent';
      expect(controller.titleController.text, 'still owned by parent');
      controller.dispose();
    },
  );
  testWidgets('internally created controller is disposed once on replacement', (
    tester,
  ) async {
    final external = MixEditorController();
    Widget app(MixEditorController? controller) =>
        ChangeNotifierProvider<ResponsiveProvider>(
          create: (_) => _Responsive(),
          child: FluentApp(
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            home: ScaffoldPage(
              content: MixEditor(
                controller: controller,
                initialSuggestions: (_) async => [],
                groupSuggestions: () async => [],
              ),
            ),
          ),
        );
    await tester.pumpWidget(app(null));
    final owned = Provider.of<MixEditorController>(
      tester.element(find.byType(InputSection)),
      listen: false,
    );
    await tester.pumpWidget(app(external));
    await tester.pump();
    expect(() => owned.addListener(() {}), throwsFlutterError);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
    external.titleController.text = 'external stays alive';
    expect(tester.takeException(), isNull);
    external.dispose();
  });
}
