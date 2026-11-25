import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/providers/router_path.dart';
import 'package:rune/providers/visualizer_lab.dart';

// A lab that is open, and nothing more.
class _OpenLab extends ChangeNotifier implements VisualizerLabProvider {
  @override
  bool get overlayOpen => true;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

// The cover wall's page on screen, where the lab is.
class _OnLabPage extends ChangeNotifier implements RouterPathProvider {
  @override
  String? path = kLabPath;

  @override
  Object? parameter;

  @override
  void update(String? x, Object? p) {}
}

void main() {
  // Whether the lab fits a window of [size], as the app's own layouts sort it.
  Future<bool> fits(WidgetTester tester, Size size) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = size;
    addTearDown(tester.view.reset);
    final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
    final responsive = ResponsiveProvider(sizes, () async => null);
    await tester.pump();
    final result = visualizerLabFits(responsive);
    responsive.dispose();
    sizes.dispose();
    // Past the size provider's wait before it saves.
    await tester.pump(const Duration(seconds: 2));
    return result;
  }

  testWidgets('a desktop, a tablet and a phone have room for the lab', (
    tester,
  ) async {
    expect(await fits(tester, const Size(1600, 900)), isTrue);
    expect(await fits(tester, const Size(768, 1024)), isTrue);
    expect(await fits(tester, const Size(390, 844)), isTrue);
    expect(await fits(tester, const Size(844, 390)), isTrue);
  });

  testWidgets('the smallest layouts have none: zune and dock upright, car and '
      'smaller on its side', (tester) async {
    expect(await fits(tester, const Size(300, 800)), isFalse);
    expect(await fits(tester, const Size(100, 800)), isFalse);
    expect(await fits(tester, const Size(1200, 320)), isFalse);
    expect(await fits(tester, const Size(1200, 120)), isFalse);
  });

  testWidgets('an open lab is hidden while the window is too small for it, '
      'and back when it grows', (tester) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1600, 900);
    addTearDown(tester.view.reset);
    final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
    final responsive = ResponsiveProvider(sizes, () async => null);
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<VisualizerLabProvider>.value(
            value: _OpenLab(),
          ),
          ChangeNotifierProvider<RouterPathProvider>.value(value: _OnLabPage()),
          ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
        ],
        child: Directionality(
          textDirection: TextDirection.ltr,
          child: Builder(
            builder: (context) =>
                Text(watchVisualizerLabMode(context) ? 'on' : 'off'),
          ),
        ),
      ),
    );
    expect(find.text('on'), findsOneWidget);
    // Down to the zune layout.
    tester.view.physicalSize = const Size(300, 800);
    await tester.pump();
    expect(find.text('off'), findsOneWidget);
    tester.view.physicalSize = const Size(1600, 900);
    await tester.pump();
    expect(find.text('on'), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
    responsive.dispose();
    sizes.dispose();
    await tester.pump(const Duration(seconds: 2));
  });
}
