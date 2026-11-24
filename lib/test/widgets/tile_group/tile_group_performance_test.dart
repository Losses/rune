import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../../widgets/start_screen/start_group.dart';
import '../../../widgets/tile_group/tile_layout.dart';
import '../../../widgets/turntile/turntile_group.dart';

// The upper bounds below are about ten times what a development machine
// measures, rounded up. They are that loose on purpose: they only fail on a
// real regression, not on a slow or busy machine. Stopwatch measures real
// time even inside the fake-async zone of testWidgets.

const _runs = 3;

Future<Duration> _best(Future<Duration> Function() run) async {
  await run();
  var best = await run();
  for (var i = 1; i < _runs; i++) {
    final time = await run();
    if (time < best) best = time;
  }
  return best;
}

Duration _timeSync(void Function() body) {
  final stopwatch = Stopwatch()..start();
  body();
  stopwatch.stop();
  return stopwatch.elapsed;
}

double _ms(Duration d) => d.inMicroseconds / 1000;

void _report(String label, Duration time, int boundMs) {
  // ignore: avoid_print
  print('perf $label: ${_ms(time).toStringAsFixed(2)} ms (bound $boundMs ms)');
  expect(time.inMicroseconds, greaterThan(0));
  expect(time, lessThan(Duration(milliseconds: boundMs)));
}

List<TileSpan> _mixedSpans(int count) {
  final random = Random(42);
  return List.generate(count, (_) {
    final roll = random.nextInt(10);
    if (roll < 7) return const TileSpan(1, 1);
    if (roll == 7) return const TileSpan(2, 1);
    if (roll == 8) return const TileSpan(1, 2);
    return const TileSpan(2, 2);
  });
}

Widget _host(Widget child) {
  return FluentTheme(
    data: FluentThemeData(),
    child: Directionality(
      textDirection: TextDirection.ltr,
      child: ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: StartScreenLayoutManager(),
        child: Align(alignment: Alignment.topLeft, child: child),
      ),
    ),
  );
}

Widget _largeGroup(List<int> items, double maxHeight) {
  return _host(
    SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      child: StartGroup<int>(
        groupIndex: 0,
        groupTitle: 'Group',
        items: items,
        itemBuilder: (context, item) => const SizedBox.expand(),
        constraints: BoxConstraints(maxHeight: maxHeight),
        direction: Axis.vertical,
      ),
    ),
  );
}

// A tall view, so the groups lay out without overflowing the default one.
void _useTallView(WidgetTester tester) {
  tester.view.physicalSize = const Size(2000, 20000);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

void main() {
  group('layoutTiles', () {
    const count = 10000;
    final ones = List.filled(count, const TileSpan(1, 1));
    final mixed = _mixedSpans(count);

    test('10000 tiles of 1 x 1, horizontal', () async {
      final time = await _best(
        () async => _timeSync(() => layoutTiles(ones, rows: 4)),
      );
      _report('layoutTiles 10000 1x1 horizontal', time, 50);
    });

    test('10000 tiles of mixed sizes, horizontal', () async {
      final time = await _best(
        () async => _timeSync(() => layoutTiles(mixed, rows: 4)),
      );
      _report('layoutTiles 10000 mixed horizontal', time, 50);
    });

    test('10000 tiles of mixed sizes, vertical', () async {
      final time = await _best(
        () async => _timeSync(
          () => layoutTiles(mixed, orientation: Axis.vertical, columns: 4),
        ),
      );
      _report('layoutTiles 10000 mixed vertical', time, 50);
    });
  });

  testWidgets('Large group of 2000 tiles lays out and re-lays out', (
    tester,
  ) async {
    _useTallView(tester);
    final items = List.generate(2000, (i) => i);
    final first = <Duration>[];
    final relayout = <Duration>[];
    for (var i = 0; i < _runs + 1; i++) {
      await tester.pumpWidget(const SizedBox());
      var stopwatch = Stopwatch()..start();
      await tester.pumpWidget(_largeGroup(items, 800));
      first.add(stopwatch.elapsed);
      stopwatch = Stopwatch()..start();
      await tester.pumpWidget(_largeGroup(items, 400));
      relayout.add(stopwatch.elapsed);
    }
    // The first round is the warm-up.
    _report(
      'StartGroup 2000 first pump',
      first.skip(1).reduce((a, b) => a < b ? a : b),
      2000,
    );
    _report(
      'StartGroup 2000 pump after maxHeight change',
      relayout.skip(1).reduce((a, b) => a < b ? a : b),
      500,
    );
  });

  testWidgets('Small group of 2000 tiles lays out', (tester) async {
    _useTallView(tester);
    final items = List.generate(2000, (i) => i);
    final times = <Duration>[];
    for (var i = 0; i < _runs + 1; i++) {
      await tester.pumpWidget(const SizedBox());
      final stopwatch = Stopwatch()..start();
      await tester.pumpWidget(
        _host(
          SingleChildScrollView(
            child: SizedBox(
              width: 400,
              child: TurntileGroup<int>(
                groupIndex: 0,
                groupTitle: 'Group',
                items: items,
                itemBuilder: (context, item) => const SizedBox.expand(),
                gridLayoutVariation: TurntileGroupGridLayoutVariation.tile,
              ),
            ),
          ),
        ),
      );
      times.add(stopwatch.elapsed);
    }
    _report(
      'TurntileGroup 2000 first pump',
      times.skip(1).reduce((a, b) => a < b ? a : b),
      2500,
    );
  });

  testWidgets('widthIn over 300 groups of 100 items', (tester) async {
    late BuildContext context;
    await tester.pumpWidget(
      _host(
        Builder(
          builder: (c) {
            context = c;
            return const SizedBox();
          },
        ),
      ),
    );
    final items = List.generate(100, (i) => i);
    final groups = List.generate(
      300,
      (i) => StartGroup<int>(
        groupIndex: i,
        groupTitle: 'Group $i',
        items: items,
        itemBuilder: (context, item) => const SizedBox.expand(),
        constraints: const BoxConstraints(maxHeight: 800),
        direction: Axis.vertical,
      ),
    );
    final time = await _best(
      () async => _timeSync(() {
        for (final group in groups) {
          group.widthIn(context);
        }
      }),
    );
    _report('widthIn 300 groups of 100 items', time, 150);
  });
}
