import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../main.dart' show cafeMode;
import '../../providers/router_path.dart';
import '../../utils/router/history.dart';
import '../../utils/router/navigation.dart';
import '../../utils/router/route_entry.dart';
import '../../utils/router/router_transition_parameter.dart';
import '../../widgets/router/router_animation.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

class _Counter extends StatefulWidget {
  const _Counter();

  @override
  State<_Counter> createState() => _CounterState();
}

class _CounterState extends State<_Counter> {
  int count = 0;

  @override
  Widget build(BuildContext context) => Text('count $count');
}

Route<void> _route(RouteSettings settings) {
  late final PageRouteBuilder<void> route;
  route = PageRouteBuilder<void>(
    settings: settings,
    transitionDuration: Duration.zero,
    reverseTransitionDuration: Duration.zero,
    pageBuilder: (context, _, _) => RouterAnimation(
      Column(
        children: [
          Text('${settings.name}'),
          if (settings.name == '/artists') const _Counter(),
        ],
      ),
      route: route,
    ),
  );
  return route;
}

Future<void> _pumpApp(WidgetTester tester) async {
  cafeMode = false;
  $router.update('/library', null);
  $history.reset();
  await tester.pumpWidget(
    FluentApp(
      navigatorKey: runeWithNavigationBarAndPlaybackControllorNavigatorKey,
      navigatorObservers: [routerAnimationObserver],
      onGenerateRoute: _route,
      initialRoute: '/library',
    ),
  );
  $push('/artists', arguments: 'list');
  await tester.pumpAndSettle();
  $push('/artists/detail', arguments: 'detail');
  await tester.pumpAndSettle();
}

Finder _inPage(String name, Type type) => find.descendant(
  of: find.ancestor(
    of: find.text(name),
    matching: find.byType(RouterAnimation),
  ),
  matching: find.byType(type),
);

void main() {
  testWidgets(
    'the revealed page plays the entrance of a page from the popped one',
    (tester) async {
      await _pumpApp(tester);

      // Opened from the library: a cross level entrance, settled by now.
      expect($pop(), isTrue);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));

      // The detail page is a child of the list, so going back is "parent".
      expect(find.text('/artists'), findsOneWidget);
      final fade = tester.widget<FadeTransition>(
        _inPage('/artists', FadeTransition),
      );
      expect(fade.opacity.value, lessThan(1));
      final scale = tester.widget<ScaleTransition>(
        _inPage('/artists', ScaleTransition),
      );
      expect(scale.scale.value, 1);
      expect(
        tester
            .widget<SlideTransition>(_inPage('/artists', SlideTransition))
            .position
            .value
            .dy,
        lessThan(0),
      );

      await tester.pumpAndSettle();
    },
  );

  testWidgets('a dialog closing over a page does not replay it', (
    tester,
  ) async {
    await _pumpApp(tester);
    final context = tester.element(find.text('/artists/detail'));

    showDialog<void>(
      context: context,
      useRootNavigator: false,
      builder: (context) => const ContentDialog(content: Text('dialog')),
    );
    await tester.pumpAndSettle();
    Navigator.of(context).pop();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    final fade = tester.widget<FadeTransition>(
      _inPage('/artists/detail', FadeTransition),
    );
    expect(fade.opacity.value, 1);
  });

  testWidgets('popping keeps the revealed page parameters object', (
    tester,
  ) async {
    await _pumpApp(tester);
    final revealed = $history.history[$history.history.length - 2];
    final original = (revealed as RouteEntry).arguments;
    final inner = (original as RouterTransitionParameter).parameters;

    $pop();
    await tester.pumpAndSettle();

    final now = getRouterTransitionParameter()!;
    expect(identical(now.parameters, inner), isTrue);
    expect(now.from, '/artists/detail');
    expect(now.to, '/artists');
    expect(identical($history.current!.arguments, original), isTrue);
  });

  testWidgets('a page revealed with another relation keeps its state', (
    tester,
  ) async {
    await _pumpApp(tester);
    final state = tester.state<_CounterState>(
      find.byType(_Counter, skipOffstage: false),
    );
    state.count = 7;

    // Opened from the library (cross level), revealed as a parent.
    $pop();
    await tester.pumpAndSettle();

    expect(tester.state<_CounterState>(find.byType(_Counter)), same(state));
    expect(state.count, 7);
  });
}
