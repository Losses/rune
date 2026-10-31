import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/lyrics/widgets/lyric_display.dart';

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
]) {
  lyrics ??= _lyrics;
  final active = [
    for (final (i, line) in lyrics.indexed)
      if (line.startTime <= timeMs && timeMs < line.endTime) i,
  ];
  return FluentApp(
    home: ChangeNotifierProvider<ResponsiveProvider>.value(
      value: _Desktop(),
      child: LyricsDisplay(
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
