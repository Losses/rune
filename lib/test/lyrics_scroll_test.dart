import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../bindings/bindings.dart';
import '../providers/responsive_providers.dart';
import '../screens/lyrics/band_screen_lyrics.dart';
import '../screens/lyrics/widgets/lyric_display.dart';
import '../screens/lyrics/widgets/lyric_line.dart';
import '../utils/playing_item.dart';

class _Desktop extends ChangeNotifier implements ResponsiveProvider {
  @override
  bool smallerOrEqualTo(DeviceType deviceType, [bool autoOrientation = true]) =>
      false;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

const _lineMs = 3000;
final _lyrics = [
  for (var i = 0; i < 60; i++)
    LyricContentLine(
      startTime: i * _lineMs,
      endTime: (i + 1) * _lineMs,
      sections: [
        LyricContentLineSection(
          startTime: i * _lineMs,
          endTime: (i + 1) * _lineMs,
          content: 'Line $i',
        ),
      ],
    ),
];

Widget _display(
  int timeMs,
  ValueNotifier<double> interpolated, [
  List<LyricContentLine>? lyrics,
  PlayingItem item = const PlayingItem.inLibrary(1),
]) {
  lyrics ??= _lyrics;
  final active = [
    for (final (i, line) in lyrics.indexed)
      if ((line.startTime <= timeMs && timeMs < line.endTime) ||
          (line.startTime == timeMs && line.endTime == timeMs))
        i,
  ];
  return FluentApp(
    home: ChangeNotifierProvider<ResponsiveProvider>.value(
      value: _Desktop(),
      child: BandScreenLyricsView(
        item: item,
        lyrics: lyrics,
        currentTimeMilliseconds: timeMs,
        activeLines: active,
        interpolatedTime: interpolated,
      ),
    ),
  );
}

double _centreOf(WidgetTester tester, int line) =>
    tester.getCenter(find.text('Line $line').first).dy;

/// Moves playback into [line] and records where that line is drawn on every
/// frame until the scroll settles.
Future<List<double>> _enterLine(
  WidgetTester tester,
  int line,
  ValueNotifier<double> interpolated,
) async {
  final timeMs = line * _lineMs + 10;
  interpolated.value = timeMs.toDouble();
  await tester.pumpWidget(_display(timeMs, interpolated));
  final track = <double>[];
  for (var frame = 0; frame < 60; frame++) {
    await tester.pump(const Duration(milliseconds: 16));
    if (find.text('Line $line').evaluate().isNotEmpty) {
      track.add(_centreOf(tester, line));
    }
  }
  return track;
}

/// The line moves towards the middle in one go: over several frames, never
/// back the other way, ending in the middle of the view.
void _expectOneSmoothScroll(List<double> track, double middle, int line) {
  expect(track, isNotEmpty, reason: 'line $line is never drawn');
  expect(
    track.toSet().length,
    greaterThan(5),
    reason: 'line $line jumps instead of scrolling: $track',
  );
  for (var i = 1; i < track.length; i++) {
    expect(
      track[i],
      lessThanOrEqualTo(track[i - 1] + 0.5),
      reason: 'line $line moves back while scrolling: $track',
    );
  }
  expect(
    track.last,
    closeTo(middle, 2),
    reason: 'line $line ends off-centre: $track',
  );
}

void main() {
  const window = Size(800, 600);

  setUp(() {
    final view =
        TestWidgetsFlutterBinding.instance.platformDispatcher.views.first;
    view.devicePixelRatio = 1;
    view.physicalSize = window;
  });

  tearDown(() {
    final view =
        TestWidgetsFlutterBinding.instance.platformDispatcher.views.first;
    view.resetPhysicalSize();
    view.resetDevicePixelRatio();
  });

  for (final keyboard in [false, true]) {
    testWidgets(
      'all plain lyrics are reachable with ${keyboard ? 'PageDown' : 'the wheel'} during playback',
      (tester) async {
        final texts = List.generate(120, (i) => 'Plain line ${i + 1}');
        final lyrics = [
          for (final text in texts)
            LyricContentLine(
              startTime: 0,
              endTime: 0,
              sections: [
                LyricContentLineSection(
                  startTime: 0,
                  endTime: 0,
                  content: text,
                ),
              ],
            ),
        ];
        final time = ValueNotifier<double>(30000);
        addTearDown(time.dispose);
        await tester.pumpWidget(_display(time.value.toInt(), time, lyrics));
        await tester.pumpAndSettle();
        if (keyboard) {
          await tester.sendKeyEvent(LogicalKeyboardKey.tab);
          await tester.pump();
        }
        final viewport = tester.getRect(find.byType(LyricsDisplay));
        final seen = <String>{};
        for (var step = 0; step < texts.length; step++) {
          for (final row in tester.widgetList<LyricLine>(
            find.byType(LyricLine),
          )) {
            if (viewport.contains(tester.getCenter(find.byWidget(row)))) {
              seen.add(row.sections.single.content);
            }
          }
          if (seen.contains(texts.last)) break;
          if (keyboard) {
            await tester.sendKeyEvent(LogicalKeyboardKey.pageDown);
          } else {
            await tester.sendEventToBinding(
              PointerScrollEvent(
                kind: PointerDeviceKind.mouse,
                position: viewport.center,
                scrollDelta: Offset(0, viewport.height / 2),
              ),
            );
          }
          time.value += 100;
          await tester.pumpWidget(_display(time.value.toInt(), time, lyrics));
          await tester.pumpAndSettle();
        }
        expect(
          seen.length,
          texts.length,
          reason: 'every lyric must be readable',
        );
        if (keyboard) {
          await tester.sendKeyEvent(LogicalKeyboardKey.pageUp);
          await tester.pumpAndSettle();
          final last = find.text(texts.last).hitTestable();
          expect(last, findsNothing);
          await tester.sendKeyEvent(LogicalKeyboardKey.end);
          await tester.pumpAndSettle();
          expect(last, findsOneWidget, reason: 'End reaches the last lyric');
        }
        // At position zero LyricsPage reports every 0/0 line as active.
        time.value = 0;
        await tester.pumpWidget(_display(0, time, lyrics));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 500));
        expect(
          find.text(texts.last).hitTestable(),
          findsOneWidget,
          reason: 'restarting playback keeps the passage being read',
        );
        if (keyboard) {
          // A duration update can normalize the same song's lyrics again.
          final normalizedAgain = List.of(lyrics);
          time.value = 500;
          await tester.pumpWidget(_display(500, time, normalizedAgain));
          await tester.pumpAndSettle();
          expect(find.text(texts.last).hitTestable(), findsOneWidget);

          await tester.pumpWidget(
            _display(500, time, normalizedAgain, const PlayingItem.inLibrary(2)),
          );
          await tester.pumpAndSettle();
          expect(
            find.text(texts.first).hitTestable(),
            findsOneWidget,
            reason: 'a different track starts at its first lyric',
          );
          expect(find.text(texts.last).hitTestable(), findsNothing);
        }
      },
    );
  }

  testWidgets('each new line scrolls once into the middle', (tester) async {
    final interpolated = ValueNotifier<double>(0);
    await tester.pumpWidget(_display(10, interpolated));
    await tester.pumpAndSettle();

    for (var line = 1; line <= 6; line++) {
      _expectOneSmoothScroll(
        await _enterLine(tester, line, interpolated),
        window.height / 2,
        line,
      );
    }
  });

  // Losses/rune#265: on Windows a minimized window is 0 x 0. Playback goes on
  // while it is minimized; after restoring, every new line used to scroll to
  // a stale position and then correct itself until the song changed.
  testWidgets('lines still scroll once after the window was minimized', (
    tester,
  ) async {
    final interpolated = ValueNotifier<double>(0);
    await tester.pumpWidget(_display(10, interpolated));
    await tester.pumpAndSettle();
    for (var line = 1; line <= 4; line++) {
      await _enterLine(tester, line, interpolated);
    }

    tester.view.physicalSize = Size.zero;
    for (var line = 5; line <= 30; line++) {
      final timeMs = line * _lineMs + 10;
      await tester.pumpWidget(_display(timeMs, interpolated));
      await tester.pump(const Duration(milliseconds: 100));
    }

    // Line 30 is placed in the middle without first going somewhere else
    tester.view.physicalSize = window;
    await tester.pumpWidget(_display(30 * _lineMs + 500, interpolated));
    final distances = <double>[];
    for (var frame = 0; frame < 30; frame++) {
      if (find.text('Line 30').evaluate().isNotEmpty) {
        distances.add((_centreOf(tester, 30) - window.height / 2).abs());
      }
      await tester.pump(const Duration(milliseconds: 16));
    }
    expect(distances, isNotEmpty);
    for (var i = 1; i < distances.length; i++) {
      expect(
        distances[i],
        lessThanOrEqualTo(distances[i - 1] + 0.5),
        reason: 'line 30 moves away while restoring: $distances',
      );
    }
    expect(
      distances.last,
      lessThan(2),
      reason: 'the current line is not in the middle after restoring',
    );

    for (var line = 31; line <= 38; line++) {
      _expectOneSmoothScroll(
        await _enterLine(tester, line, interpolated),
        window.height / 2,
        line,
      );
    }
  });

  // Lyrics with nothing left to show (still loading, or played past) must
  // not keep the lines of the previous lyrics as the place to scroll to.
  testWidgets('new lyrics do not keep the old lines as the target', (
    tester,
  ) async {
    final interpolated = ValueNotifier<double>(0);
    await tester.pumpWidget(_display(10, interpolated));
    await tester.pumpAndSettle();
    await _enterLine(tester, 30, interpolated);

    final short = _lyrics.sublist(0, 3);
    await tester.pumpWidget(_display(40 * _lineMs, interpolated, const []));
    await tester.pumpWidget(_display(40 * _lineMs, interpolated, short));
    // A new list is built when the window comes back
    tester.view.physicalSize = Size.zero;
    await tester.pump();
    tester.view.physicalSize = window;
    await tester.pumpWidget(_display(40 * _lineMs, interpolated, short));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(find.text('Line 2'), findsOneWidget);
  });
}
