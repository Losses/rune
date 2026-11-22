import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/collection/utils/collection_data_provider.dart';
import 'package:rune/utils/query_list.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'package:rune/widgets/navigation_bar/page_content_frame.dart';
import 'package:rune/widgets/start_screen/start_screen.dart';
import 'package:rune/widgets/start_screen/utils/group.dart';
import 'package:rune/widgets/start_screen/utils/internal_collection.dart';
import 'package:rune/widgets/turntile/turntile_screen.dart';

class _UnreadablePage extends Fake implements List<Group<InternalCollection>> {
  @override
  int get length => throw StateError('Collection page unavailable');
}

void main() {
  for (final (screen, size) in const [
    (TurntileScreen(), Size(320, 620)),
    (StartScreen(), Size(1000, 700)),
  ]) {
    testWidgets('Retry resumes a group jump in ${screen.runtimeType}', (
      tester,
    ) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = size;
      addTearDown(tester.view.reset);
      const type = CollectionType.album;
      final groups = [
        for (final title in 'ABCDEFG'.split(''))
          Group(
            groupTitle: title,
            items: List.generate(
              12,
              (i) => InternalCollection(
                id: title.codeUnitAt(0) * 100 + i,
                name: 'Album $title $i',
                collectionType: type,
                coverArtMap: {},
                queries: const QueryList([]),
                readonly: true,
              ),
            ),
          ),
      ];
      final cache = CollectionCache();
      addTearDown(cache.clearAll);
      cache.cacheSummary(type, groups);
      cache.cachePage(type, 0, groups.sublist(0, 3));
      cache.cachePage(type, 1, _UnreadablePage());
      cache.cachePage(type, 2, groups.sublist(6));
      final data = CollectionDataProvider(collectionType: type);
      addTearDown(data.dispose);
      await data.fetchData();
      final content = ValueNotifier<Widget>(screen);
      addTearDown(content.dispose);
      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider.value(value: data),
            ChangeNotifierProvider(
              create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
            ),
            ChangeNotifierProvider(
              create: (context) => ResponsiveProvider(
                context.read<ScreenSizeProvider>(),
                () async => null,
              ),
            ),
          ],
          child: FluentApp(
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            home: RevealEffectContext(
              child: PageContentFrame(
                child: ValueListenableBuilder<Widget>(
                  valueListenable: content,
                  builder: (_, page, _) => page,
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      // Select G before the list's initial debounced fetch can run.
      await tester.tap(find.text('A'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 32));
      await tester.tap(find.widgetWithText(Button, 'G'));
      await tester.pump();
      final failure = data.pageError;
      expect(failure, isA<StateError>());
      cache.cachePage(type, 1, groups.sublist(3, 6));
      await tester.pump(const Duration(milliseconds: 300));
      expect(data.pageError, same(failure));
      expect(data.items.map((group) => group.groupTitle), ['A', 'B', 'C']);
      final retry = find.widgetWithText(Button, 'Retry');
      await tester.scrollUntilVisible(retry, 300);
      await tester.tap(retry);
      for (var frame = 0; frame < 12; frame++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      expect(data.items.map((group) => group.groupTitle), 'ABCDEFG'.split(''));
      expect(data.pageError, isNull);
      expect(find.text('G').hitTestable(), findsOneWidget);

      // The picker can outlive the layout that opened it when a window resizes.
      await tester.tap(find.text('G'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      content.value = const SizedBox();
      await tester.pump();
      await tester.tap(find.widgetWithText(Button, 'A'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(tester.takeException(), isNull);
    });
  }
}
