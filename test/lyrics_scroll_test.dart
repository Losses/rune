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

Widget _display(int timeMs, ValueNotifier<double> interpolated) {
  final active = [
    for (final (i, line) in _lyrics.indexed)
      if (line.startTime <= timeMs && timeMs < line.endTime) i,
  ];
  return FluentApp(
    home: ChangeNotifierProvider<ResponsiveProvider>.value(
      value: _Desktop(),
      child: LyricsDisplay(
        lyrics: _lyrics,
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

/// The line moves towards the middle in one go: it never moves back the
/// other way, and it ends in the middle of the view.
void _expectOneSmoothScroll(List<double> track, double middle, int line) {
  expect(track, isNotEmpty, reason: 'line $line is never drawn');
  for (var i = 1; i < track.length; i++) {
    expect(track[i], lessThanOrEqualTo(track[i - 1] + 0.5),
        reason: 'line $line moves back while scrolling: $track');
  }
  expect(track.last, closeTo(middle, 2),
      reason: 'line $line ends off-centre: $track');
}

void main() {
  const window = Size(800, 600);

  setUp(() {
    final view = TestWidgetsFlutterBinding.instance.platformDispatcher.views.first;
    view.devicePixelRatio = 1;
    view.physicalSize = window;
  });

  tearDown(() {
    final view = TestWidgetsFlutterBinding.instance.platformDispatcher.views.first;
    view.resetPhysicalSize();
    view.resetDevicePixelRatio();
  });

  testWidgets('each new line scrolls once into the middle', (tester) async {
    final interpolated = ValueNotifier<double>(0);
    await tester.pumpWidget(_display(10, interpolated));
    await tester.pumpAndSettle();

    for (var line = 1; line <= 6; line++) {
      _expectOneSmoothScroll(
          await _enterLine(tester, line, interpolated), window.height / 2, line);
    }
  });

  // Losses/rune#265: on Windows a minimized window is 0 x 0. Playback goes on
  // while it is minimized; after restoring, every new line used to scroll to
  // a stale position and then correct itself until the song changed.
  testWidgets('lines still scroll once after the window was minimized',
      (tester) async {
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

    tester.view.physicalSize = window;
    await tester.pumpWidget(_display(30 * _lineMs + 500, interpolated));
    await tester.pumpAndSettle();
    expect(_centreOf(tester, 30), closeTo(window.height / 2, 2),
        reason: 'the current line is not in the middle after restoring');

    for (var line = 31; line <= 38; line++) {
      _expectOneSmoothScroll(
          await _enterLine(tester, line, interpolated), window.height / 2, line);
    }
  });
}
