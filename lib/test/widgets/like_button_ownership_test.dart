import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../providers/library_open_state.dart';
import '../../utils/playing_item.dart';
import '../../widgets/playback_controller/like_button.dart';

const a = PlayingItem.libraryItem(libraryId: 'library', fileUuid: 'a');
const b = PlayingItem.libraryItem(libraryId: 'library', fileUuid: 'b');

void main() {
  for (final returnToA in [false, true]) {
    for (final pendingToggle in [false, true]) {
      testWidgets(
        'late ${pendingToggle ? "toggle" : "read"} after ${returnToA ? "A-B-A" : "A-B"} preserves current heart and tap',
        (tester) async {
          final pending = Completer<bool>();
          final writes = <(PlayingItem, bool)>[];
          var reads = 0;
          var holdToggle = pendingToggle;
          Future<bool> fetch(PlayingItem item) {
            if (++reads == 1 && !pendingToggle) return pending.future;
            return Future.value(false);
          }

          Future<bool> toggle(PlayingItem item, bool liked) {
            writes.add((item, liked));
            if (holdToggle) {
              holdToggle = false;
              return pending.future;
            }
            return Future.value(liked);
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
          expect(tester.widget<Icon>(find.byIcon(Symbols.favorite)).fill, 0);
          await tester.tap(find.byIcon(Symbols.favorite));
          await tester.pump();
          expect(writes.last, (returnToA ? a : b, true));
          expect(tester.widget<Icon>(find.byIcon(Symbols.favorite)).fill, 1);
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
