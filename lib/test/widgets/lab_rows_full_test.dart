import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_filter.dart';
import 'package:rune/widgets/visualizer/lab/lab_timeline.dart';
import 'package:rune/widgets/visualizer/lab/lab_views.dart';

const _columns = [
  LabColumn(
    id: 'shot',
    name: 'Shot',
    group: 'Director',
    kind: LabColumnKind.category,
  ),
  LabColumn(
    id: 'drive',
    name: 'Drive',
    group: 'Signals',
    kind: LabColumnKind.number,
  ),
];

void main() {
  final pane = find.byKey(const ValueKey('lab-views-pane'));
  final views = find.byWidgetPredicate(
    (w) => w is Tooltip && w.message == 'Views',
  );

  Future<LabRowsModel> open(
    WidgetTester tester,
    Size size, {
    FakeViewPadding padding = FakeViewPadding.zero,
  }) async {
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1;
    tester.view.padding = padding;
    tester.view.viewPadding = padding;
    addTearDown(tester.view.reset);
    final model = LabRowsModel();
    await tester.pumpWidget(
      FluentApp(
        builder: (context, child) => RevealEffectContext(child: child!),
        home: ScaffoldPage(
          padding: EdgeInsets.zero,
          content: LabTimeline(
            data: LabTimelineData(length: 60, columns: _columns),
            rows: model,
            onSeek: (_) {},
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.tap(find.byIcon(Symbols.filter_list));
    await tester.pumpAndSettle();
    return model;
  }

  Rect dialog(WidgetTester tester) =>
      tester.getRect(find.byType(LabRowsDialog));

  testWidgets('a phone upright: the dialog is the window, the pane opens '
      'over the list', (tester) async {
    final model = await open(tester, const Size(390, 844));
    expect(dialog(tester), const Rect.fromLTWH(0, 0, 390, 844));
    expect(pane, findsNothing);
    expect(views, findsOneWidget);

    await tester.tap(views);
    await tester.pumpAndSettle();
    expect(pane, findsOneWidget);
    final p = tester.getRect(pane);
    expect(p.left, 0);
    expect(p.width, 232);
    // Over the list, which keeps its width.
    expect(find.text('Director'), findsWidgets);

    await tester.tap(find.text('Music'));
    await tester.pumpAndSettle();
    expect(model.currentName, 'Music');
    expect(pane, findsNothing);

    await tester.tap(views);
    await tester.pumpAndSettle();
    expect(pane, findsOneWidget);
    await tester.tapAt(const Offset(350, 400));
    await tester.pumpAndSettle();
    expect(pane, findsNothing);
    // The dialog is still open, and the close button stays at the right.
    expect(find.byType(LabRowsDialog), findsOneWidget);
    expect(tester.getRect(find.byIcon(Symbols.close)).right, lessThan(390));
    expect(tester.getRect(find.byIcon(Symbols.close)).right, greaterThan(350));
  });

  testWidgets('the pane names a new view in its place over the list', (
    tester,
  ) async {
    final model = await open(tester, const Size(390, 844));
    await tester.tap(views);
    await tester.pumpAndSettle();
    await tester.tap(find.text('New View'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextBox), 'Mine');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(model.ownViews.single.name, 'Mine');
  });

  testWidgets('a phone on its side: the window, both panes side by side', (
    tester,
  ) async {
    await open(tester, const Size(844, 390));
    expect(dialog(tester), const Rect.fromLTWH(0, 0, 844, 390));
    expect(views, findsNothing);
    expect(pane, findsOneWidget);
    expect(tester.getRect(pane).right, 232);
  });

  testWidgets(
    'a desktop window keeps the 800 wide dialog and no Views button',
    (tester) async {
      await open(tester, const Size(1600, 900));
      // Fluent's transition rests a little under the constraint.
      expect(dialog(tester).width, inInclusiveRange(700, 800));
      expect(views, findsNothing);
      expect(pane, findsOneWidget);
    },
  );

  testWidgets('the header and the list stay inside the safe area', (
    tester,
  ) async {
    await open(
      tester,
      const Size(390, 844),
      padding: const FakeViewPadding(top: 47, bottom: 34),
    );
    final d = dialog(tester);
    expect(d.top, greaterThanOrEqualTo(47));
    expect(d.bottom, lessThanOrEqualTo(844 - 34));
    final menu = tester.getRect(find.byIcon(Symbols.menu));
    expect(menu.top, greaterThanOrEqualTo(47));
    final list = tester.getRect(find.byType(CustomScrollView));
    expect(list.bottom, lessThanOrEqualTo(844 - 34));
  });
}
