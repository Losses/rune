import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../providers/library_open_state.dart';
import '../../utils/playing_item.dart';
import '../../widgets/playback_controller/like_button.dart';
import '../../widgets/rune_clickable.dart';

const a = PlayingItem.libraryItem(libraryId: 'library', fileUuid: 'a');
const b = PlayingItem.libraryItem(libraryId: 'library', fileUuid: 'b');

void main() {
  testWidgets('independent files have no Like button or liked-state request', (
    tester,
  ) async {
    const independent = PlayingItem.independentFile('/music/independent.ogg');
    final reads = <PlayingItem>[];
    final writes = <(PlayingItem, bool)>[];
    var persisted = false;

    Future<void> show(PlayingItem item) async {
      await tester.pumpWidget(
        ChangeNotifierProvider<LibraryOpenStateProvider>(
          create: (_) => _LibraryOpenStateProvider(),
          child: FluentApp(
            home: LikeButton(
              item: item,
              fetchLikedStatus: (requested) async {
                reads.add(requested);
                return persisted;
              },
              setLikedStatus: (requested, liked) async {
                writes.add((requested, liked));
                return persisted = liked;
              },
            ),
          ),
        ),
      );
      await tester.pump();
    }

    final heart = find.byIcon(Symbols.favorite);
    await show(independent);
    expect(heart, findsNothing);
    expect(reads, isEmpty);

    await show(a);
    expect(heart, findsOneWidget);
    expect(reads, [a]);
    final likeControl = find.ancestor(
      of: heart,
      matching: find.byType(RuneClickable),
    );
    final previousToggle = tester.widget<RuneClickable>(likeControl).onPressed!;
    await tester.tap(heart);
    await tester.pump();
    expect(writes, [(a, true)]);

    await show(independent);
    expect(heart, findsNothing);
    expect(reads, [a]);
    previousToggle();
    await tester.pump();
    expect(writes, [(a, true)]);

    await show(a);
    expect(tester.widget<Icon>(heart).fill, 1);
    expect(reads, [a, a]);
  });
  for (final returnToA in [false, true]) {
    for (final pendingToggle in [false, true]) {
      testWidgets(
        'late ${pendingToggle ? "toggle" : "read"} after ${returnToA ? "A-B-A" : "A-B"} preserves current heart and tap',
        (tester) async {
          final pending = Completer<bool>();
          final writes = <(PlayingItem, bool)>[];
          final persisted = <PlayingItem, bool>{a: false, b: false};
          var reads = 0;
          var holdToggle = pendingToggle;
          Future<bool> fetch(PlayingItem item) {
            if (++reads == 1 && !pendingToggle) return pending.future;
            return Future.value(persisted[item]!);
          }

          Future<bool> toggle(PlayingItem item, bool liked) async {
            writes.add((item, liked));
            if (holdToggle) {
              holdToggle = false;
              await pending.future;
            }
            persisted[item] = liked;
            return liked;
          }

          Future<void> show(PlayingItem item) async {
            await tester.pumpWidget(
              ChangeNotifierProvider<LibraryOpenStateProvider>(
                create: (_) => _LibraryOpenStateProvider(),
                child: FluentApp(
                  home: LikeButton(
                    item: item,
                    fetchLikedStatus: fetch,
                    setLikedStatus: toggle,
                  ),
                ),
              ),
            );
            await tester.pump();
          }

          await show(a);
          final originalState = tester.state(find.byType(LikeButton));
          if (pendingToggle) {
            await tester.tap(find.byIcon(Symbols.favorite));
            await tester.pump();
            expect(writes, [(a, true)]);
          }
          await show(b);
          if (returnToA) await show(a);
          expect(tester.state(find.byType(LikeButton)), same(originalState));
          expect(tester.widget<Icon>(find.byIcon(Symbols.favorite)).fill, 0);

          pending.complete(true);
          await tester.pump();
          final currentLiked = returnToA && pendingToggle;
          expect(
            tester.widget<Icon>(find.byIcon(Symbols.favorite)).fill,
            currentLiked ? 1 : 0,
          );
          await tester.tap(find.byIcon(Symbols.favorite));
          await tester.pump();
          expect(writes.last, (returnToA ? a : b, !currentLiked));
          expect(
            tester.widget<Icon>(find.byIcon(Symbols.favorite)).fill,
            currentLiked ? 0 : 1,
          );
        },
      );
    }
  }

  testWidgets('a toggle supersedes a pending read and serial toggles work', (
    tester,
  ) async {
    final read = Completer<bool>();
    final writes = <bool>[];
    await tester.pumpWidget(
      ChangeNotifierProvider<LibraryOpenStateProvider>(
        create: (_) => _LibraryOpenStateProvider(),
        child: FluentApp(
          home: LikeButton(
            item: a,
            fetchLikedStatus: (_) => read.future,
            setLikedStatus: (_, liked) async {
              writes.add(liked);
              return liked;
            },
          ),
        ),
      ),
    );
    await tester.tap(find.byIcon(Symbols.favorite));
    await tester.pump();
    expect(tester.widget<Icon>(find.byIcon(Symbols.favorite)).fill, 1);
    read.complete(false);
    await tester.pump();
    expect(tester.widget<Icon>(find.byIcon(Symbols.favorite)).fill, 1);
    await tester.tap(find.byIcon(Symbols.favorite));
    await tester.pump();
    expect(writes, [true, false]);
    expect(tester.widget<Icon>(find.byIcon(Symbols.favorite)).fill, 0);
  });
}

class _LibraryOpenStateProvider extends ChangeNotifier
    implements LibraryOpenStateProvider {
  @override
  bool get isReadOnlyReplica => false;

  @override
  LibraryRecordSummary? get openRecord => null;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
