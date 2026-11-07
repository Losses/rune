import 'dart:convert';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/widgets/ax_reveal/ax_reveal.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'package:rune/widgets/hover_opacity.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_menu.dart';
import 'package:rune/widgets/visualizer/lab/lab_display.dart';
import 'package:rune/widgets/visualizer/lab/lab_filter.dart';
import 'package:rune/widgets/visualizer/lab/lab_rows.dart';
import 'package:rune/widgets/visualizer/lab/lab_series.dart';
import 'package:rune/widgets/visualizer/lab/lab_timeline.dart';
import 'package:rune/widgets/visualizer/lab/lab_views.dart';

const _columns = [
  labSectionsColumn,
  labMarksColumn,
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
  LabColumn(
    id: 'low',
    name: 'Low',
    group: 'Signals',
    kind: LabColumnKind.number,
  ),
  LabColumn(
    id: 'band_low',
    name: 'Band Low',
    group: 'Signals',
    kind: LabColumnKind.number,
    plan: LabPlanAvailability.no,
  ),
  LabColumn(
    id: 'input:glow',
    name: 'Glow',
    group: 'Inputs',
    kind: LabColumnKind.number,
    binds: ['drive', 'low'],
  ),
  LabColumn(
    id: 'input:solo',
    name: 'Solo',
    group: 'Inputs',
    kind: LabColumnKind.number,
  ),
];

LabColumn _col(String id) => _columns.firstWhere((c) => c.id == id);

void main() {
  group('selection', () {
    test('a value of an older format gives the defaults', () {
      final old = jsonEncode({
        'shot': {'p': 0, 'a': 1},
        'age': {'p': 1, 'a': 0},
      });
      expect(LabRowSelection.fromObject(jsonDecode(old)), isNull);
      expect(LabRowSelection.fromJson(old), LabRowSelection.defaults);
      expect(
        LabRowSelection.fromJson('{"v":1,"rows":{}}'),
        LabRowSelection.defaults,
      );
      expect(LabRowSelection.fromJson('{"v":2}'), LabRowSelection.defaults);
    });

    test('the built-in views are made once for a list of columns', () {
      final views = labBuiltInViews(_columns);
      expect(identical(labBuiltInViews(_columns), views), isTrue);
      final other = [..._columns];
      expect(identical(labBuiltInViews(other), views), isFalse);
      expect(labBuiltInViews(other).map((v) => v.name), labBuiltInNames);
    });

    test('a missing or garbled value gives the defaults', () {
      expect(LabRowSelection.fromJson(null), LabRowSelection.defaults);
      expect(LabRowSelection.fromJson('{'), LabRowSelection.defaults);
      expect(LabRowSelection.fromJson('[1]'), LabRowSelection.defaults);
      expect(LabRowSelection.defaults.pins, ['sections', 'marks']);
    });

    test('version 2 survives JSON', () {
      final s = LabRowSelection.defaults
          .pin('drive')
          .setPart('shot', plan: false);
      final json = jsonDecode(s.toJson()) as Map;
      expect(json['v'], 2);
      final back = LabRowSelection.fromJson(s.toJson());
      expect(back, s);
      expect(back.pins, ['sections', 'marks', 'drive']);
      expect(back.planOn('shot'), isFalse);
      expect(back.hashCode, s.hashCode);
    });

    test('selections that show the same are equal', () {
      const a = LabRowSelection({'x': (plan: false, actual: false)});
      const b = LabRowSelection({});
      expect(a, b);
      expect(a.hashCode, b.hashCode);
      expect(
        const LabRowSelection({'input:a': (plan: false, actual: true)}),
        isNot(const LabRowSelection({})),
      );
    });

    test('withVisible turns on every strip a column has, or none', () {
      const s = LabRowSelection({});
      final on = s.withVisible(_col('shot'), on: true);
      expect(on.isShown(_col('shot')), isTrue);
      expect(on.planOn('shot') && on.actualOn('shot'), isTrue);
      final band = s.withVisible(_col('band_low'), on: true);
      expect(band.planOn('band_low'), isFalse);
      expect(band.actualOn('band_low'), isTrue);
      final sections = s.withVisible(_col('sections'), on: true);
      expect(sections.planOn('sections'), isTrue);
      expect(sections.actualOn('sections'), isFalse);
      expect(
        on.withVisible(_col('shot'), on: false).isShown(_col('shot')),
        isFalse,
      );
    });

    test('a row draws the strips it has on and can show', () {
      final all = LabRowSelection.all(_columns);
      expect(all.showsPlan(_col('shot')), isTrue);
      expect(all.showsActual(_col('shot')), isTrue);
      // Only an actual strip.
      expect(all.showsPlan(_col('band_low')), isFalse);
      expect(all.showsActual(_col('band_low')), isTrue);
      // Only a plan strip.
      expect(all.showsActual(_col('sections')), isFalse);
      final only = all.setPart('shot', plan: false, actual: true);
      expect(only.showsPlan(_col('shot')), isFalse);
      expect(only.showsActual(_col('shot')), isTrue);
    });

    test('pins keep their order', () {
      var s = LabRowSelection.defaults;
      expect(s.isPinned('marks'), isTrue);
      s = s.pin('drive').pin('shot').pin('drive');
      expect(s.pins, ['sections', 'marks', 'drive', 'shot']);
      s = s.movePin('shot', -2);
      expect(s.pins, ['sections', 'shot', 'marks', 'drive']);
      s = s.movePin('sections', -1).movePin('drive', 5);
      expect(s.pins, ['sections', 'shot', 'marks', 'drive']);
      s = s.movePin('sections', 9);
      expect(s.pins, ['shot', 'marks', 'drive', 'sections']);
      s = s.unpin('marks').unpin('nothing');
      expect(s.pins, ['shot', 'drive', 'sections']);
      expect(s.isPinned('marks'), isFalse);
    });

    test('related columns go both ways', () {
      final ids = [
        for (final c in LabRowSelection.related(_col('input:glow'), _columns))
          c.id,
      ];
      expect(ids, ['drive', 'low']);
      final inputs = [
        for (final c in LabRowSelection.related(_col('drive'), _columns)) c.id,
      ];
      expect(inputs, ['input:glow']);
      expect(LabRowSelection.related(_col('shot'), _columns), isEmpty);
      expect(LabRowSelection.related(_col('input:solo'), _columns), isEmpty);
    });
  });

  group('views', () {
    LabRowView built(String name) =>
        labBuiltInViews(_columns).firstWhere((v) => v.name == name);

    test('the built-in views are made of the columns', () {
      final names = [for (final v in labBuiltInViews(_columns)) v.name];
      expect(names, ['Overview', 'Director', 'Music', 'Inputs', 'All']);
      expect(built('Overview').selection, LabRowSelection.defaults);

      final director = built('Director').selection;
      expect(director.isShown(_col('shot')), isTrue);
      expect(director.isShown(_col('drive')), isFalse);
      expect(director.isShown(_col('input:glow')), isFalse);
      expect(director.pins, ['sections', 'marks']);

      final music = built('Music').selection;
      expect(music.isShown(_col('drive')), isTrue);
      expect(music.isShown(_col('shot')), isFalse);
      expect(music.isShown(_col('band_low')), isTrue);
      expect(music.showsPlan(_col('band_low')), isFalse);

      final inputs = built('Inputs').selection;
      expect(inputs.isShown(_col('input:glow')), isTrue);
      expect(inputs.isShown(_col('input:solo')), isTrue);
      expect(inputs.isShown(_col('drive')), isTrue);
      expect(inputs.isShown(_col('low')), isTrue);
      expect(inputs.isShown(_col('shot')), isFalse);
      expect(inputs.isShown(_col('sections')), isTrue);

      final all = built('All').selection;
      expect(_columns.every(all.isShown), isTrue);
    });

    test('a view without inputs leaves out those of another scene', () {
      const other = LabColumn(
        id: 'input:other',
        name: 'Other',
        group: 'Inputs',
        kind: LabColumnKind.number,
      );
      final views = {for (final v in labBuiltInViews(_columns)) v.name: v};
      for (final name in ['Director', 'Music']) {
        final selection = LabRowSelection.fromJson(
          views[name]!.selection.toJson(),
        );
        expect(selection.isShown(other), isFalse, reason: name);
      }
      for (final name in ['Overview', 'Inputs', 'All']) {
        final selection = LabRowSelection.fromJson(
          views[name]!.selection.toJson(),
        );
        expect(selection.isShown(other), isTrue, reason: name);
      }
    });

    test('the views of the user are read and written', () {
      final own = [
        LabRowView(
          name: 'Mine',
          selection: LabRowSelection.defaults.pin('shot'),
        ),
        const LabRowView(name: 'Other', selection: LabRowSelection({})),
      ];
      final back = labViewsFromJson(labViewsToJson(own));
      expect([for (final v in back) v.name], ['Mine', 'Other']);
      expect(back.first.selection.pins, ['sections', 'marks', 'shot']);
      expect(back.first.builtIn, isFalse);
    });

    test('entries that cannot be used are skipped', () {
      final json = jsonEncode([
        {'name': 'Fine', 'selection': LabRowSelection.defaults.toMap()},
        {'name': 'fine ', 'selection': LabRowSelection.defaults.toMap()},
        {'name': 'overview', 'selection': LabRowSelection.defaults.toMap()},
        {'name': '  ', 'selection': LabRowSelection.defaults.toMap()},
        {'name': 'No Rows'},
        {'selection': LabRowSelection.defaults.toMap()},
        'text',
        {'name': 'Last', 'selection': LabRowSelection.defaults.toMap()},
      ]);
      expect(
        [for (final v in labViewsFromJson(json)) v.name],
        ['Fine', 'Last'],
      );
      expect(labViewsFromJson('{'), isEmpty);
      expect(labViewsFromJson(null), isEmpty);
    });

    test('names are unique, ignoring case and spaces', () {
      expect(labViewNameError('', ['A']), isNotNull);
      expect(labViewNameError(' a ', ['A']), isNotNull);
      expect(labViewNameError('b', ['A']), isNull);
      expect(labViewNameError('a', ['A'], except: 'A'), isNull);
    });

    test('the model tells when the rows were edited', () {
      final saved = <String, String>{};
      final model = LabRowsModel(onSave: (k, v) => saved[k] = v);
      expect(model.currentName, labOverviewName);
      expect(model.isEdited(_columns), isFalse);

      model.setSelection(model.selection.unpin('marks'));
      expect(model.isEdited(_columns), isTrue);
      expect(saved.containsKey(LabRowsModel.rowsKey), isTrue);

      model.resetView(_columns);
      expect(model.isEdited(_columns), isFalse);

      model.applyView('Director', _columns);
      expect(model.currentName, 'Director');
      expect(saved[LabRowsModel.viewKey], 'Director');
      expect(model.isEdited(_columns), isFalse);
    });

    test('a built-in view follows the catalogue, an edited one does not', () {
      const other = [
        labSectionsColumn,
        labMarksColumn,
        LabColumn(
          id: 'drive',
          name: 'Drive',
          group: 'Signals',
          kind: LabColumnKind.number,
        ),
        LabColumn(
          id: 'low',
          name: 'Low',
          group: 'Signals',
          kind: LabColumnKind.number,
        ),
        LabColumn(
          id: 'input:solo',
          name: 'Solo',
          group: 'Inputs',
          kind: LabColumnKind.number,
          binds: ['low'],
        ),
      ];
      LabRowView inputsFor(List<LabColumn> columns) =>
          labBuiltInViews(columns).firstWhere((v) => v.name == 'Inputs');

      final model = LabRowsModel();
      model.applyView('Inputs', _columns);
      model.catalogueChanged(_columns, other);
      expect(model.currentName, 'Inputs');
      expect(model.selection, inputsFor(other).selection);
      expect(model.isEdited(other), isFalse);
      expect(model.selection.planOn('drive'), isFalse);
      expect(model.selection.actualOn('low'), isTrue);

      // An edited view stays.
      final edited = LabRowsModel()..applyView('Inputs', _columns);
      edited.setSelection(edited.selection.unpin('marks'));
      final kept = edited.selection;
      edited.catalogueChanged(_columns, other);
      expect(edited.selection, kept);

      // So does a view of the user's.
      final own = LabRowsModel()
        ..applyView('All', _columns)
        ..saveView('Mine', _columns);
      final mine = own.selection;
      own.catalogueChanged(_columns, other);
      expect(own.currentName, 'Mine');
      expect(own.selection, mine);
    });

    test('hideEmpty belongs to the display, not to a view', () {
      final model = LabRowsModel(onSave: (_, _) {});
      model.setDisplay(model.display.withHideEmpty(false));
      expect(model.isEdited(_columns), isFalse);
      model.applyView('Director', _columns);
      expect(model.display.hideEmpty, isFalse);
      model.setDisplay(model.display.withHideEmpty(true));
      expect(model.isEdited(_columns), isFalse);
      model.applyView(labOverviewName, _columns);
      expect(model.display.hideEmpty, isTrue);
    });

    test('the model saves, renames, updates and deletes views', () {
      final model = LabRowsModel(onSave: (_, _) {});
      model.applyView('Director', _columns);
      model.setSelection(model.selection.pin('shot'));
      model.saveView('  Mine ', _columns);
      expect(model.currentName, 'Mine');
      expect(model.isEdited(_columns), isFalse);

      // A taken name is refused, and so is a built-in's.
      model.saveView('mine', _columns);
      model.saveView('Director', _columns);
      expect(model.ownViews, hasLength(1));

      model.setSelection(model.selection.unpin('shot'));
      expect(model.isEdited(_columns), isTrue);
      model.updateView('Mine');
      expect(model.isEdited(_columns), isFalse);

      model.renameView('Mine', 'Better', _columns);
      expect(model.currentName, 'Better');
      model.renameView('Director', 'Nope', _columns);
      model.updateView('Director');
      model.deleteView('Director');
      expect(model.views(_columns).any((v) => v.name == 'Nope'), isFalse);
      expect(model.views(_columns).first.name, labOverviewName);

      model.deleteView('Better');
      expect(model.ownViews, isEmpty);
      expect(model.currentName, labOverviewName);
      expect(model.isEdited(_columns), isTrue);
    });

    test('switching views keeps the display and does not mark Edited', () {
      final saved = <String, String>{};
      final model = LabRowsModel(onSave: (k, v) => saved[k] = v);
      final display = const LabDisplay()
          .withStripMode(LabStripMode.plan)
          .withAllLines(true)
          .withExpanded('drive', on: true);
      model.setDisplay(display);
      expect(saved[LabRowsModel.displayKey], display.toJson());
      expect(model.isEdited(_columns), isFalse);

      model.applyView('Director', _columns);
      expect(model.display, display);
      expect(model.isEdited(_columns), isFalse);
      model.resetView(_columns);
      expect(model.display, display);
      model.saveView('Mine', _columns);
      model.deleteView('Mine');
      expect(model.display, display);

      model.resetView(_columns);
      model.setDisplay(const LabDisplay());
      expect(model.isEdited(_columns), isFalse);
      var notified = 0;
      model.addListener(() => notified++);
      model.setDisplay(const LabDisplay());
      expect(notified, 0);
      model.setDisplay(display);
      expect(notified, 1);
    });

    test('a view does not hold the display', () {
      final model = LabRowsModel();
      model.setDisplay(const LabDisplay().withAllLines(true));
      model.saveView('Mine', _columns);
      expect(
        labViewsToJson(model.ownViews),
        isNot(anyOf(contains('strips'), contains('lines'))),
      );
    });

    test('the model loads the saved display', () {
      final model = LabRowsModel();
      final display = const LabDisplay().withStripMode(LabStripMode.actual);
      model.load(
        rows: LabRowSelection.defaults.toJson(),
        display: display.toJson(),
      );
      expect(model.display, display);
    });

    test('old or unknown saved values load as the defaults', () {
      final old = jsonEncode({
        'shot': {'p': 0, 'a': 1},
        'strips': 'plan',
        'lines': true,
        'styles': {'low': 'line'},
      });
      final model = LabRowsModel();
      model.load(
        rows: old,
        views: jsonEncode([
          {'name': 'Mine', 'selection': jsonDecode(old)},
          'junk',
        ]),
        view: 'Nothing Like It',
        display: 'not json',
      );
      expect(model.selection, LabRowSelection.defaults);
      expect(model.display, const LabDisplay());
      expect(model.ownViews, isEmpty);

      model.load(rows: 'not json', views: '{', display: '[1]');
      expect(model.selection, LabRowSelection.defaults);
      expect(model.display, const LabDisplay());
      expect(model.ownViews, isEmpty);
      model.load();
      expect(model.display, const LabDisplay());
    });

    test('the model loads what was saved', () {
      final model = LabRowsModel();
      model.load(
        rows: LabRowSelection.all(_columns).toJson(),
        views: labViewsToJson([
          const LabRowView(name: 'Mine', selection: LabRowSelection({})),
        ]),
        view: 'Mine',
      );
      expect(model.currentName, 'Mine');
      expect(model.ownViews.single.name, 'Mine');
      expect(model.isEdited(_columns), isTrue);
    });
  });

  group('list', () {
    test('groups every row in catalogue order, and leaves the pinned out '
        'when asked', () {
      final s = LabRowSelection.defaults.pin('drive');
      List<String> ids({bool leavePinned = false}) => [
        for (final (_, rows) in labRowGroups(
          _columns,
          s,
          leavePinned: leavePinned,
        ))
          for (final c in rows) c.id,
      ];
      expect(ids(), [for (final c in _columns) c.id]);
      expect(ids(leavePinned: true), isNot(contains('drive')));
      expect(
        [for (final (name, _) in labRowGroups(_columns, s)) name],
        ['Plan & Actual', 'Director', 'Signals', 'Inputs'],
      );
    });
  });

  group('toolbar', () {
    testWidgets('the rows button is an icon that names the view, and no '
        'picker stands apart', (tester) async {
      tester.view.physicalSize = const Size(1200, 700);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final model = LabRowsModel();
      await tester.pumpWidget(
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
          child: FluentApp(
            // As the app's root: the reveal of its items needs it.
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
        ),
      );
      await tester.pump();
      Finder tip(String message) =>
          find.byWidgetPredicate((w) => w is Tooltip && w.message == message);
      // No words on the toolbar's right: the icons, and the view in a tip.
      expect(find.text('Overview'), findsNothing);
      expect(find.text('Display'), findsNothing);
      expect(tip('Rows: Overview'), findsOneWidget);
      expect(find.byType(LabChoiceButton<String>), findsNothing);
      model.setSelection(model.selection.withVisible(_col('shot'), on: false));
      await tester.pump();
      expect(tip('Rows: Overview (Edited)'), findsOneWidget);
      model.applyView('Director', _columns);
      await tester.pump();
      expect(tip('Rows: Director'), findsOneWidget);
      // Follow and the zoom are on the left of Display and the rows.
      expect(
        tester.getCenter(find.text('Follow')).dx,
        lessThan(tester.getCenter(find.byIcon(Symbols.tune)).dx),
      );
      expect(
        tester.getCenter(find.byIcon(Symbols.tune)).dx,
        lessThan(tester.getCenter(find.byIcon(Symbols.filter_list)).dx),
      );
    });
  });

  group('dialog', () {
    Future<LabRowsModel> open(
      WidgetTester tester, {
      LabRowsModel? model,
    }) async {
      tester.view.physicalSize = const Size(1200, 1400);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      model ??= LabRowsModel();
      await tester.pumpWidget(
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
          child: FluentApp(
            // As the app's root: the reveal of its items needs it.
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
        ),
      );
      await tester.pump();
      await tester.tap(find.byIcon(Symbols.filter_list));
      await tester.pumpAndSettle();
      return model;
    }

    // The name of a row in the dialog.
    Finder name(String text) =>
        find.textContaining(text, findRichText: true).last;

    // A row of the dialog, by its name.
    Finder rows(String name) => find.text(name, findRichText: true);

    Finder tip(String message) =>
        find.byWidgetPredicate((w) => w is Tooltip && w.message == message);
    // A group's heading in the list: its name, which may give way to room.
    bool says(Widget w, String text) =>
        w is Text && w.data == text ||
        w is Flexible && w.child is Text && (w.child as Text).data == text;
    Finder heading(String name) => find.descendant(
      of: find.byType(CustomScrollView),
      matching: find.byWidgetPredicate(
        (w) => w is Row && w.children.any((c) => says(c, name)),
      ),
    );

    // The heading a row is listed under: the nearest above it.
    String groupOf(WidgetTester tester, String row) {
      final top = tester.getRect(rows(row)).top;
      String? under;
      var at = double.negativeInfinity;
      for (final name in ['Pinned', 'Director', 'Signals', 'Inputs']) {
        final found = heading(name);
        if (found.evaluate().isEmpty) continue;
        final y = tester.getRect(found).top;
        if (y < top && y > at) (under, at) = (name, y);
      }
      return under!;
    }

    Future<TestGesture> hover(WidgetTester tester, Finder at) async {
      final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await gesture.addPointer(location: Offset.zero);
      addTearDown(gesture.removePointer);
      await gesture.moveTo(tester.getCenter(at));
      await tester.pump();
      return gesture;
    }

    testWidgets('opens from the toolbar and changes the rows', (tester) async {
      final model = await open(tester);
      expect(model.selection.isShown(_col('shot')), isTrue);
      expect(find.byType(LabRowsDialog), findsOneWidget);
      await tester.tap(name('Shot'));
      await tester.pump();
      expect(model.selection.isShown(_col('shot')), isFalse);

      // Escape closes it.
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byType(LabRowsDialog), findsNothing);
    });

    testWidgets('no title, no search and no filter: the views on the left, '
        'every row on the right, headed by the view\'s name', (tester) async {
      final model = await open(tester);
      expect(find.text('Rows'), findsNothing);
      expect(find.byType(TextBox), findsNothing);
      expect(find.byWidgetPredicate((w) => w is LabChoiceButton), findsNothing);
      for (final c in _columns) {
        expect(find.text(c.name), findsWidgets, reason: c.id);
      }
      final pane = tester.getRect(find.byKey(const ValueKey('lab-views-pane')));
      // The view's name heads the list, right of the pane.
      Rect heading() => tester.getRect(find.text(model.currentName).last);
      expect(heading().left, greaterThan(pane.right));
      model.applyView('Music', _columns);
      await tester.pumpAndSettle();
      expect(find.text('Music'), findsNWidgets(2));
      expect(heading().left, greaterThan(pane.right));
      // The close button stands at the right edge, where the strips end,
      // edited or not (within the dialog's scale as it opens).
      final dialog = tester.getRect(find.byType(LabRowsDialog));
      double closeRight() => tester.getRect(tip('Close')).right;
      expect(closeRight(), closeTo(dialog.right - 16, 1));
      model.setSelection(model.selection.withVisible(_col('drive'), on: false));
      await tester.pumpAndSettle();
      expect(model.isEdited(_columns), isTrue);
      expect(closeRight(), closeTo(dialog.right - 16, 1));
      await tester.tap(tip('Close'));
      await tester.pumpAndSettle();
      expect(find.byType(LabRowsDialog), findsNothing);
    });

    testWidgets('the views sit on acrylic and the rows on a solid ground, as '
        'Groove\'s pane and content do', (tester) async {
      await open(tester);
      final dialog = find.byType(LabRowsDialog);
      expect(
        tester.widget(find.byKey(const ValueKey('lab-views-pane'))),
        isA<Acrylic>(),
      );
      final theme = FluentTheme.of(tester.element(dialog));
      final grounds = [
        for (final d in tester.widgetList<DecoratedBox>(
          find.descendant(of: dialog, matching: find.byType(DecoratedBox)),
        ))
          if (d.decoration case BoxDecoration(:final color?)) color,
      ];
      expect(grounds, contains(theme.micaBackgroundColor));
      // The dialog paints nothing over what the acrylic shows of the page.
      final box = tester.widget<ContentDialog>(find.byType(ContentDialog));
      expect((box.style!.decoration! as BoxDecoration).color, isNull);
    });

    testWidgets('a row is one of fluent\'s list items: its name alone and a '
        'plain checkbox for each strip', (tester) async {
      final model = await open(tester);
      final row = find.ancestor(
        of: name('Shot'),
        matching: find.byType(ListTile),
      );
      expect(tester.widget<ListTile>(row).subtitle, isNull);
      final boxes = tester
          .widgetList<Checkbox>(
            find.descendant(of: row, matching: find.byType(Checkbox)),
          )
          .toList();
      expect(boxes, hasLength(2));
      // fluent's own checkboxes, as they come: no style of ours.
      expect(boxes.every((b) => b.style == null), isTrue);
      // The strips' names are not on the row but in each group's heading,
      // over the columns of its rows' checkboxes.
      expect(boxes.every((b) => b.content == null), isTrue);
      expect(
        find.descendant(of: row, matching: find.text('Plan')),
        findsNothing,
      );
      // Pinned, Director, Signals and Inputs.
      expect(find.text('Plan'), findsNWidgets(4));
      expect(find.text('Actual'), findsNWidgets(4));
      // The Director group's heading, over Shot.
      final plan = tester.getRect(find.text('Plan').at(1));
      final actual = tester.getRect(find.text('Actual').at(1));
      final rowBoxes = find.descendant(
        of: row,
        matching: find.byType(Checkbox),
      );
      expect(tester.getCenter(rowBoxes.at(0)).dx, closeTo(plan.center.dx, 0.5));
      expect(
        tester.getCenter(rowBoxes.at(1)).dx,
        closeTo(actual.center.dx, 0.5),
      );
      expect(plan.bottom, lessThan(tester.getRect(rowBoxes.at(0)).top));
      expect(boxes.map((b) => b.semanticLabel), ['Plan', 'Actual']);
      // A click on the row shows or hides all of it.
      await tester.tap(name('Shot'));
      await tester.pumpAndSettle();
      expect(model.selection.isShown(_col('shot')), isFalse);
    });

    testWidgets('groups stand apart, and what says less is written as the '
        'app\'s settings write their subtitles', (tester) async {
      await open(tester);
      // Room between the last row of Pinned and the Director heading.
      final marks = tester.getRect(
        find.ancestor(of: name('Marks'), matching: find.byType(ListTile)),
      );
      final director = tester.getRect(find.text('Director').last);
      expect(director.top - marks.bottom, greaterThanOrEqualTo(16));
      // A strip's name over its column in the app's subtitle colour.
      final theme = FluentTheme.of(tester.element(find.byType(LabRowsDialog)));
      final secondary = theme.inactiveColor.withAlpha(160);
      Color? colorOf(Finder f) => tester.widget<Text>(f).style?.color;
      expect(colorOf(find.text('Plan').first), secondary);
    });

    testWidgets('a heading says only its name, and sticks right under the '
        'view\'s name with no band for the rows to show through', (
      tester,
    ) async {
      await open(tester);
      // No count of rows beside any heading.
      for (final name in ['Pinned', 'Director', 'Signals', 'Inputs']) {
        expect(heading(name), findsOneWidget, reason: name);
      }
      expect(find.textContaining(RegExp(r'^\d+(/\d+)?$')), findsNothing);
      // In a window short enough for the list to scroll, scrolled a little
      // into Signals: its heading sticks at the top of the list, where the
      // rows go under it.
      tester.view.physicalSize = const Size(1200, 360);
      await tester.pumpAndSettle();
      final list = tester.getRect(find.byType(CustomScrollView));
      final scroll = tester.state<ScrollableState>(
        find
            .descendant(
              of: find.byType(CustomScrollView),
              matching: find.byType(Scrollable),
            )
            .first,
      );
      final to = tester.getRect(heading('Signals')).top - list.top + 10;
      expect(to, lessThan(scroll.position.maxScrollExtent));
      scroll.position.jumpTo(to);
      await tester.pumpAndSettle();
      final title = tester.getRect(heading('Signals'));
      expect(title.top - list.top, inInclusiveRange(0, 8));
      // The groups still stand apart where they are not stuck.
      final inputs = tester.getRect(heading('Inputs'));
      final last = tester.getRect(
        find.ancestor(of: rows('Band Low'), matching: find.byType(ListTile)),
      );
      expect(inputs.top - last.bottom, greaterThanOrEqualTo(16));
    });

    testWidgets('a row says nothing under its name, and keeps its height '
        'when its buttons show on hover', (tester) async {
      await open(tester);
      Finder row(String text) =>
          find.ancestor(of: name(text), matching: find.byType(ListTile));
      // The catalogue's own rows, and one of the hub's.
      for (final text in ['Sections', 'Marks', 'Shot', 'Glow']) {
        expect(
          tester.widget<ListTile>(row(text)).subtitle,
          isNull,
          reason: text,
        );
      }
      final height = tester.getSize(row('Sections')).height;
      expect(height, tester.getSize(row('Shot')).height);
      await hover(tester, name('Sections'));
      expect(
        find.descendant(of: row('Sections'), matching: find.byType(IconButton)),
        findsWidgets,
      );
      expect(tester.getSize(row('Sections')).height, height);
    });

    testWidgets('the rows going under a stuck heading leave no seam at the '
        'top of the list, where the dialog\'s scale puts its edge between '
        'two pixels', (tester) async {
      await open(tester);
      // A window short enough for the list to scroll.
      tester.view.physicalSize = const Size(1200, 360);
      await tester.pumpAndSettle();
      final list = tester.getRect(find.byType(CustomScrollView));
      expect(list.top, isNot(list.top.roundToDouble()));
      // Pinned's heading stuck, and the name of its first row across the
      // list's top edge, under the heading.
      final scroll = tester.state<ScrollableState>(
        find
            .descendant(
              of: find.byType(CustomScrollView),
              matching: find.byType(Scrollable),
            )
            .first,
      );
      final to = tester.getRect(rows('Sections')).center.dy - list.top;
      expect(to, lessThan(scroll.position.maxScrollExtent));
      scroll.position.jumpTo(to);
      await tester.pumpAndSettle();
      final size = tester.view.physicalSize;
      final pixels = (await tester.runAsync(() async {
        final layer =
            tester.binding.renderViews.first.debugLayer! as OffsetLayer;
        final image = await layer.toImage(Offset.zero & size);
        return image.toByteData(format: ui.ImageByteFormat.rawRgba);
      }))!;
      List<int> at(int x, int y) {
        final i = (y * size.width.toInt() + x) * 4;
        return [for (var c = 0; c < 4; c++) pixels.getUint8(i + c)];
      }

      // The ground, in the stuck heading under the edge.
      final edge = list.top.floor();
      final ground = at((list.left + 300).toInt(), edge + 3);
      for (var x = (list.left + 16).toInt(); x < list.left + 160; x++) {
        final p = at(x, edge);
        for (var c = 0; c < 3; c++) {
          expect(
            (p[c] - ground[c]).abs(),
            lessThanOrEqualTo(2),
            reason: 'x $x',
          );
        }
      }
    });

    testWidgets('a strip a row has not is a checkbox that cannot be ticked, '
        'saying why', (tester) async {
      await open(tester);
      final row = find.ancestor(
        of: name('Band Low'),
        matching: find.byType(ListTile),
      );
      final plan = tester.widget<Checkbox>(
        find.descendant(of: row, matching: find.byType(Checkbox)).first,
      );
      expect(plan.onChanged, isNull);
      expect(
        find.descendant(
          of: row,
          matching: find.byWidgetPredicate(
            (w) => w is Tooltip && w.message == 'No Plan for Live Audio',
          ),
        ),
        findsOneWidget,
      );
    });

    testWidgets('its tips are named as the app\'s are: each word capitalised '
        'but the small ones, and no full stop', (tester) async {
      final model = await open(tester);
      model.saveView('Bass Check', _columns);
      model.setSelection(model.selection.withVisible(_col('shot'), on: false));
      await tester.pumpAndSettle();
      const small = {
        'a',
        'an',
        'the',
        'and',
        'or',
        'for',
        'to',
        'of',
        'in',
        'on',
        'at',
        'by',
        'as',
        'with',
        'from',
      };
      final tips = tester
          .widgetList<Tooltip>(find.byType(Tooltip))
          .map((t) => t.message)
          .whereType<String>()
          .toSet();
      expect(tips, containsAll(['Close', 'Reset View', 'More Options']));
      expect(tips, containsAll(['No Plan for Live Audio', 'Zoom In']));
      for (final tip in tips) {
        expect(tip.endsWith('.'), isFalse, reason: tip);
        final words = tip.split(' ');
        for (final (i, word) in words.indexed) {
          final w = word.replaceAll(RegExp(r'[^A-Za-z]'), '');
          if (w.isEmpty) continue;
          final lower = w[0] == w[0].toLowerCase();
          final isSmall = i > 0 && i < words.length - 1 && small.contains(w);
          expect(lower, isSmall, reason: '$tip: $word');
        }
      }
    });

    testWidgets('a click on the name of a row shows or hides it', (
      tester,
    ) async {
      final model = await open(tester);
      await tester.tap(name('Shot'));
      await tester.pumpAndSettle();
      expect(model.selection.isShown(_col('shot')), isFalse);
      expect(model.isEdited(_columns), isTrue);
      await tester.tap(name('Shot'));
      await tester.pumpAndSettle();
      expect(model.selection.isShown(_col('shot')), isTrue);
    });

    testWidgets('the Plan and Actual checkboxes set the strips of a row', (
      tester,
    ) async {
      final model = await open(tester);
      final row = find.ancestor(
        of: name('Shot'),
        matching: find.byType(ListTile),
      );
      Finder box(String label) => find.descendant(
        of: row,
        matching: find.byWidgetPredicate(
          (w) => w is Checkbox && w.semanticLabel == label,
        ),
      );
      expect(model.selection.planOn('shot'), isTrue);
      await tester.tap(box('Plan'));
      await tester.pumpAndSettle();
      expect(model.selection.planOn('shot'), isFalse);
      expect(model.selection.actualOn('shot'), isTrue);
      await tester.tap(box('Actual'));
      await tester.pumpAndSettle();
      expect(model.selection.isShown(_col('shot')), isFalse);
      await tester.tap(box('Plan'));
      await tester.pumpAndSettle();
      expect(model.selection.planOn('shot'), isTrue);
    });

    testWidgets('New View names the view in its place, with no dialog over '
        'the dialog, and keeps it under a name that is free', (tester) async {
      final model = await open(tester);
      expect(heading('Pinned'), findsOneWidget);
      final pane = find.byKey(const ValueKey('lab-views-pane'));
      Finder box() => find.descendant(of: pane, matching: find.byType(TextBox));

      await tester.tap(find.text('New View'));
      await tester.pumpAndSettle();
      expect(find.byType(ContentDialog), findsOneWidget);
      expect(box(), findsOneWidget);
      expect(find.text('New View'), findsNothing);

      await tester.enterText(box(), 'director');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      // The box tells it, as fluent's form boxes tell what is wrong.
      expect(
        find.descendant(
          of: find.byType(TextFormBox),
          matching: find.text('A view with this name exists.'),
        ),
        findsOneWidget,
      );
      expect(model.ownViews, isEmpty);
      // Once told, it goes as the name is mended.
      await tester.enterText(box(), 'Mine');
      await tester.pump();
      expect(find.text('A view with this name exists.'), findsNothing);

      await tester.enterText(box(), 'Mine');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(model.ownViews.single.name, 'Mine');
      expect(model.currentName, 'Mine');
      expect(box(), findsNothing);
      expect(find.text('New View'), findsOneWidget);
    });

    testWidgets('Escape leaves the name, not the dialog', (tester) async {
      final model = await open(tester);
      await tester.tap(find.text('New View'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextBox), 'Half');
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byType(TextBox), findsNothing);
      expect(find.byType(LabRowsDialog), findsOneWidget);
      expect(model.ownViews, isEmpty);
    });

    testWidgets('Rename names a view of the user\'s in its place', (
      tester,
    ) async {
      final model = await open(tester);
      model.saveView('Bass Check', _columns);
      await tester.pumpAndSettle();
      await hover(tester, find.text('Bass Check').first);
      await tester.tap(find.byIcon(Symbols.more_horiz));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Rename'));
      await tester.pumpAndSettle();
      final box = find.byType(TextBox);
      expect(tester.widget<TextBox>(box).controller!.text, 'Bass Check');
      await tester.enterText(box, 'Low End');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(model.ownViews.single.name, 'Low End');
      expect(find.byType(TextBox), findsNothing);
    });

    testWidgets('the views are fluent\'s selectable list items, the chosen '
        'one selected', (tester) async {
      final model = await open(tester);
      final pane = find.byKey(const ValueKey('lab-views-pane'));
      final tiles = tester
          .widgetList<ListTile>(
            find.descendant(of: pane, matching: find.byType(ListTile)),
          )
          .toList();
      // Five views and New View.
      expect(tiles, hasLength(6));
      expect(
        tiles.where((t) => t.selectionMode == ListTileSelectionMode.single),
        hasLength(5),
      );
      final chosen = tester.widget<ListTile>(
        find.ancestor(
          of: find.descendant(of: pane, matching: find.text(model.currentName)),
          matching: find.byType(ListTile),
        ),
      );
      expect(chosen.selected, isTrue);
      // Each has the app's reveal, which lights it under the pointer.
      for (final tile
          in find
              .descendant(of: pane, matching: find.byType(ListTile))
              .evaluate()) {
        expect(
          find.ancestor(
            of: find.byWidget(tile.widget),
            matching: find.byType(AxReveal0),
          ),
          findsOneWidget,
        );
      }
    });

    testWidgets('the views have icons and no row count; the user\'s own '
        'come after a line, then New View', (tester) async {
      final model = await open(tester);
      for (final icon in const [
        Symbols.dashboard,
        Symbols.videocam,
        Symbols.music_note,
        Symbols.instant_mix,
        Symbols.view_list,
      ]) {
        expect(find.byIcon(icon), findsOneWidget);
      }
      expect(find.textContaining('shown'), findsNothing);
      expect(find.text('VIEWS'), findsNothing);
      expect(find.text('MINE'), findsNothing);
      final pane = find.byKey(const ValueKey('lab-views-pane'));
      double top(Finder f) => tester.getRect(f).top;
      final line = find.descendant(of: pane, matching: find.byType(Divider));
      final add = find.text('New View');
      expect(top(line), greaterThan(top(find.text('All'))));
      expect(top(add), greaterThan(top(line)));

      model.saveView('Bass check', _columns);
      await tester.pumpAndSettle();
      expect(find.byIcon(Symbols.bookmark), findsOneWidget);
      final mine = find.descendant(of: pane, matching: find.text('Bass check'));
      expect(top(mine), greaterThan(top(line)));
      expect(top(add), greaterThan(top(mine)));
    });

    testWidgets('an edited view has a way back on its row in the pane, an '
        'icon alone', (tester) async {
      final model = await open(tester);
      expect(tip('Reset View'), findsNothing);
      await tester.tap(name('Shot'));
      await tester.pumpAndSettle();
      // No word for it, and nothing by the name over the list.
      expect(find.text('Edited'), findsNothing);
      final pane = find.byKey(const ValueKey('lab-views-pane'));
      final view = find.ancestor(
        of: find.descendant(of: pane, matching: find.text('Overview')),
        matching: find.byType(ListTile),
      );
      final reset = find.descendant(of: view, matching: tip('Reset View'));
      expect(reset, findsOneWidget);
      expect(
        find.descendant(of: reset, matching: find.byIcon(Symbols.undo)),
        findsOneWidget,
      );
      // At the row's right side, past its name.
      final title = tester.getRect(
        find.descendant(of: view, matching: find.text('Overview')),
      );
      expect(tester.getRect(reset).left, greaterThan(title.right));
      expect(
        tester.getRect(reset).right,
        greaterThan(tester.getRect(view).right - 24),
      );
      // The item keeps its height.
      final director = find.ancestor(
        of: find.descendant(of: pane, matching: find.text('Director')),
        matching: find.byType(ListTile),
      );
      expect(
        tester.getSize(view).height,
        closeTo(tester.getSize(director).height, 0.5),
      );
      await tester.tap(reset);
      await tester.pumpAndSettle();
      expect(model.isEdited(_columns), isFalse);
      expect(model.selection.isShown(_col('shot')), isTrue);
      expect(tip('Reset View'), findsNothing);
    });

    testWidgets('a pinned column is listed once, under Pinned', (tester) async {
      final model = await open(tester);
      // Sections and Marks are pinned and are not in a Pinned-free group.
      expect(rows('Sections'), findsOneWidget);
      expect(rows('Marks'), findsOneWidget);
      expect(groupOf(tester, 'Drive'), 'Signals');
      model.setSelection(model.selection.pin('drive'));
      await tester.pump();
      expect(rows('Drive'), findsOneWidget);
      expect(groupOf(tester, 'Drive'), 'Pinned');
      model.setSelection(model.selection.unpin('drive'));
      await tester.pump();
      expect(rows('Drive'), findsOneWidget);
      expect(groupOf(tester, 'Drive'), 'Signals');
    });

    testWidgets('a heading\'s title answers the pointer as the app\'s own '
        'group titles do', (tester) async {
      await open(tester);
      final title = find.ancestor(
        of: heading('Director'),
        matching: find.byType(HoverOpacity),
      );
      expect(title, findsOneWidget);
      double opacity() => tester
          .widget<AnimatedOpacity>(
            find.descendant(of: title, matching: find.byType(AnimatedOpacity)),
          )
          .opacity;
      expect(opacity(), 0.7);
      await hover(tester, heading('Director'));
      await tester.pumpAndSettle();
      expect(opacity(), 1);
      // Pinned, which does not collapse, looks the same at rest.
      expect(
        find.ancestor(
          of: heading('Pinned'),
          matching: find.byType(HoverOpacity),
        ),
        findsOneWidget,
      );
    });

    testWidgets('a heading shows all or hides all of its rows', (tester) async {
      final model = await open(tester);
      final signals = heading('Signals');
      expect(signals, findsOneWidget);
      expect(find.text('Hide All'), findsNothing);
      await hover(tester, signals);
      await tester.tap(find.text('Hide All'));
      await tester.pumpAndSettle();
      for (final id in ['drive', 'low', 'band_low']) {
        expect(model.selection.isShown(_col(id)), isFalse, reason: id);
      }
      expect(model.selection.isShown(_col('shot')), isTrue);
      await tester.tap(find.text('Show All'));
      await tester.pumpAndSettle();
      for (final id in ['drive', 'low', 'band_low']) {
        expect(model.selection.isShown(_col(id)), isTrue, reason: id);
      }

      // A click on the heading collapses the group.
      await tester.tap(find.text('Signals'));
      await tester.pumpAndSettle();
      expect(rows('Drive'), findsNothing);
      expect(find.byIcon(Symbols.chevron_right), findsOneWidget);
    });

    group('context menus, for a finger and a right click', () {
      // The context menu is a long press on mobile platforms and a right
      // click on desktop ones, as the app's other lists have it.
      Future<void> press(WidgetTester tester, Finder at, {bool touch = true}) {
        return touch
            ? tester.longPress(at)
            : tester.tap(
                at,
                buttons: kSecondaryButton,
                kind: PointerDeviceKind.mouse,
              );
      }

      // The platform has to be put back before the test ends, which is
      // before its tear downs.
      Future<void> onPlatform(bool touch, Future<void> Function() body) async {
        debugDefaultTargetPlatformOverride = touch
            ? TargetPlatform.android
            : TargetPlatform.macOS;
        try {
          await body();
        } finally {
          debugDefaultTargetPlatformOverride = null;
        }
      }

      for (final touch in [true, false]) {
        final how = touch ? 'a long press' : 'a right click';

        testWidgets('$how on a row has its hover buttons as a menu', (
          tester,
        ) async {
          await onPlatform(touch, () async {
            final model = await open(tester);
            await press(tester, name('Glow'), touch: touch);
            await tester.pumpAndSettle();
            expect(find.text('Show Related Rows'), findsOneWidget);
            expect(find.text('Pin to Top'), findsOneWidget);
            await tester.tap(find.text('Show Related Rows'));
            await tester.pumpAndSettle();
            expect(model.selection.isShown(_col('drive')), isTrue);

            await press(tester, name('Shot'), touch: touch);
            await tester.pumpAndSettle();
            await tester.tap(find.text('Pin to Top'));
            await tester.pumpAndSettle();
            expect(model.selection.isPinned('shot'), isTrue);

            // A pinned row has Unpin, and the moves that apply.
            model.setSelection(model.selection.pin('drive'));
            await tester.pumpAndSettle();
            await press(tester, name('Shot'), touch: touch);
            await tester.pumpAndSettle();
            expect(find.text('Move Up'), findsOneWidget);
            expect(find.text('Move Down'), findsOneWidget);
            await tester.tap(find.text('Move Down'));
            await tester.pumpAndSettle();
            expect(model.selection.pins.last, 'shot');
            await press(tester, name('Shot'), touch: touch);
            await tester.pumpAndSettle();
            expect(find.text('Move Down'), findsNothing);
            await tester.tap(find.text('Unpin'));
            await tester.pumpAndSettle();
            expect(model.selection.isPinned('shot'), isFalse);
          });
        });

        testWidgets('$how on a heading shows or hides all of its rows', (
          tester,
        ) async {
          await onPlatform(touch, () async {
            final model = await open(tester);
            await press(tester, find.text('Signals'), touch: touch);
            await tester.pumpAndSettle();
            await tester.tap(find.text('Hide All'));
            await tester.pumpAndSettle();
            expect(model.selection.isShown(_col('drive')), isFalse);
            await press(tester, find.text('Signals'), touch: touch);
            await tester.pumpAndSettle();
            await tester.tap(find.text('Show All'));
            await tester.pumpAndSettle();
            expect(model.selection.isShown(_col('drive')), isTrue);
          });
        });

        testWidgets('$how on a view of the user\'s has the ⋯ menu\'s items', (
          tester,
        ) async {
          await onPlatform(touch, () async {
            final model = await open(tester);
            model.saveView('Bass Check', _columns);
            await tester.pumpAndSettle();
            // A built-in view has no menu.
            await press(tester, find.text('Director').first, touch: touch);
            await tester.pumpAndSettle();
            expect(find.text('Delete'), findsNothing);
            await press(tester, find.text('Bass Check').first, touch: touch);
            await tester.pumpAndSettle();
            expect(find.text('Rename'), findsOneWidget);
            expect(find.text('Update'), findsOneWidget);
            await tester.tap(find.text('Delete'));
            await tester.pumpAndSettle();
            expect(model.ownViews, isEmpty);
          });
        });
      }
    });

    testWidgets('a pinned row offers only the moves it can make, as fluent\'s '
        'icon buttons a little apart', (tester) async {
      await open(tester);
      Finder action(String tip) =>
          find.byWidgetPredicate((w) => w is Tooltip && w.message == tip);
      // Sections is the first pinned row: it goes no higher.
      final hovering = await hover(tester, name('Sections'));
      await tester.pump();
      expect(action('Move Up'), findsNothing);
      expect(action('Move Down'), findsOneWidget);
      // Marks is the last: it goes no lower.
      await hovering.moveTo(tester.getCenter(name('Marks')));
      await tester.pump();
      expect(action('Move Up'), findsOneWidget);
      expect(action('Move Down'), findsNothing);
      final buttons = [
        for (final tip in ['Move Up', 'Unpin'])
          find.descendant(of: action(tip), matching: find.byType(IconButton)),
      ];
      for (final button in buttons) {
        expect(tester.widget<IconButton>(button).style, isNull);
      }
      // 4 apart, as the dialog's open scale draws it.
      expect(
        tester.getRect(buttons[1]).left - tester.getRect(buttons[0]).right,
        greaterThan(3),
      );
    });

    testWidgets('the pin shows on hover and pins the row', (tester) async {
      final model = await open(tester);
      expect(tip('Pin to Top'), findsNothing);
      await hover(tester, name('Shot'));
      await tester.tap(tip('Pin to Top'));
      await tester.pumpAndSettle();
      expect(model.selection.isPinned('shot'), isTrue);
      expect(groupOf(tester, 'Shot'), 'Pinned');
    });
  });
}
