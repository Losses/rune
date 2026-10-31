import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/search/search.dart';
import 'package:rune/utils/storage_key_manager.dart';
import 'package:rune/utils/settings_manager.dart';

class _Responsive extends ChangeNotifier implements ResponsiveProvider {
  @override
  bool smallerOrEqualTo(DeviceType type, [bool autoOrientation = true]) =>
      false;
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Screen extends ChangeNotifier implements ScreenSizeProvider {
  @override
  Size get screenSize => const Size(1000, 700);
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  late Directory storage;
  setUpAll(() async {
    storage = await Directory.systemTemp.createTemp('rune-search-test');
    if (Platform.isWindows) {
      stdout.writeln('RUNE_SEARCH_TEST_STORAGE=${storage.path}');
    }
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('plugins.flutter.io/path_provider'),
          (_) async => storage.path,
        );
    StorageKeyManager.initialize(null);
    await SettingsManager().getValue<List<dynamic>?>('search_suggestions');
  });
  tearDownAll(() async {
    // GetStorage retains an open file handle until process exit on Windows.
    // The Windows test runner removes the reported directory after exit.
    if (!Platform.isWindows) await storage.delete(recursive: true);
  });

  testWidgets(
    'search preserves focused composing input across tablet breakpoint',
    (tester) async {
      final responsive = _Responsive();
      final screen = _Screen();
      Widget app(DeviceType device) => MultiProvider(
        providers: [
          ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
          ChangeNotifierProvider<ScreenSizeProvider>.value(value: screen),
        ],
        child: FluentApp(
          builder: (context, child) => MediaQuery(
            data: MediaQuery.of(
              context,
            ).copyWith(devicePixelRatio: device == DeviceType.tablet ? 2 : 1),
            child: child!,
          ),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: SearchPageImplementation(deviceType: device),
        ),
      );
      await tester.pumpWidget(app(DeviceType.tv));
      await tester.pump();
      final editable = tester.state<EditableTextState>(
        find.byType(EditableText),
      );
      final field = tester.widget<EditableText>(find.byType(EditableText));
      field.focusNode.requestFocus();
      await tester.pump();
      tester.testTextInput.updateEditingValue(
        const TextEditingValue(
          text: 'nihao',
          selection: TextSelection.collapsed(offset: 5),
          composing: TextRange(start: 0, end: 5),
        ),
      );
      await tester.pump();
      final suggestion = tester.state<AutoSuggestBoxState<String>>(
        find.byType(AutoSuggestBox<String>),
      );
      suggestion.showOverlay();
      await tester.pump();
      expect(suggestion.isOverlayVisible, isTrue);
      final clients = tester.testTextInput.log
          .where((call) => call.method == 'TextInput.setClient')
          .length;
      await tester.pumpWidget(app(DeviceType.tablet));
      await tester.pump();
      expect(
        tester.state<EditableTextState>(find.byType(EditableText)),
        same(editable),
      );
      expect(suggestion.isOverlayVisible, isFalse);
      expect(field.focusNode.hasFocus, isTrue);
      expect(
        field.controller.value.composing,
        const TextRange(start: 0, end: 5),
      );
      expect(field.controller.text, 'nihao');
      expect(
        tester.testTextInput.log
            .where((call) => call.method == 'TextInput.setClient')
            .length,
        clients,
      );
      await tester.pumpWidget(app(DeviceType.tv));
      await tester.pump();
      expect(
        tester.state<EditableTextState>(find.byType(EditableText)),
        same(editable),
      );
      expect(field.focusNode.hasFocus, isTrue);
      expect(
        field.controller.value.composing,
        const TextRange(start: 0, end: 5),
      );
      expect(
        tester.testTextInput.log
            .where((call) => call.method == 'TextInput.setClient')
            .length,
        clients,
      );
      await tester.pumpWidget(const SizedBox());
      responsive.dispose();
      screen.dispose();
    },
  );
}
