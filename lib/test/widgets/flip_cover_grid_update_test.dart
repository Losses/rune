import 'package:flip_card/flip_card.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../widgets/tile/cover_art.dart';
import '../../widgets/tile/flip_cover_grid.dart';

// The grid a search result keeps while its cover paths are published again.
Widget _grid(List<String> paths) => FluentTheme(
  data: FluentThemeData(),
  child: Directionality(
    textDirection: TextDirection.ltr,
    child: Center(
      child: SizedBox(
        width: 120,
        height: 120,
        child: FlipCoverGrid(id: 'Search result', paths: paths),
      ),
    ),
  ),
);

void main() {
  testWidgets('a grid built with a cached cover shows it', (tester) async {
    await tester.pumpWidget(_grid(const ['/covers/a.png']));

    expect(tester.takeException(), isNull);
    expect(find.byType(FlipCard), findsOneWidget);
    expect(
      tester.widgetList<CoverArt>(find.byType(CoverArt)).map((art) => art.path),
      ['/covers/a.png', '/covers/a.png'],
    );
  });

  testWidgets('a grid that stays without covers keeps its placeholder', (
    tester,
  ) async {
    await tester.pumpWidget(_grid(const []));
    await tester.pumpWidget(_grid(const []));

    expect(tester.takeException(), isNull);
    expect(find.byType(BoringAvatar), findsOneWidget);
    expect(find.byType(FlipCard), findsNothing);
  });

  testWidgets('a grid built without covers shows the covers that arrive', (
    tester,
  ) async {
    await tester.pumpWidget(_grid(const []));
    await tester.pumpWidget(_grid(const ['/covers/a.png']));

    expect(tester.takeException(), isNull);
    expect(find.byType(FlipCard), findsOneWidget);
    expect(find.byType(CoverArt), findsNWidgets(2));
  });

  testWidgets('a grid whose cover is replaced shows the new cover', (
    tester,
  ) async {
    await tester.pumpWidget(_grid(const ['/covers/a.png']));
    await tester.pumpWidget(_grid(const ['/covers/b.png']));

    expect(tester.takeException(), isNull);
    expect(find.byType(FlipCard), findsOneWidget);
    expect(
      tester.widgetList<CoverArt>(find.byType(CoverArt)).map((art) => art.path),
      ['/covers/b.png', '/covers/b.png'],
    );
  });

  testWidgets('a grid that gains covers lays out and flips the new tiles', (
    tester,
  ) async {
    await tester.pumpWidget(_grid(const ['/covers/a.png', '/covers/b.png']));
    await tester.pumpWidget(
      _grid(const [
        '/covers/a.png',
        '/covers/b.png',
        '/covers/c.png',
        '/covers/d.png',
        '/covers/e.png',
      ]),
    );
    expect(tester.takeException(), isNull);
    expect(find.byType(FlipCard), findsNWidgets(4));

    await tester.pump(const Duration(seconds: 9));
    await tester.pump(const Duration(seconds: 1));
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('a grid whose covers stop flipping leaves no flip timer behind', (
    tester,
  ) async {
    await tester.pumpWidget(_grid(const ['/covers/a.png', '/covers/b.png']));
    await tester.pumpWidget(_grid(const ['/covers/a.png']));
    expect(tester.takeException(), isNull);

    // The test framework fails a test that ends with a timer still pending.
    await tester.pumpWidget(const SizedBox());
    expect(tester.takeException(), isNull);
  });
}
