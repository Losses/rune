import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/playlist.dart';
import '../../providers/request_capabilities.dart';
import '../../widgets/playback_controller/queue.dart';

PlaylistEntry queueEntry(PlaylistUnavailableReason? reason) => PlaylistEntry(
  0,
  PlaylistItem(
    unavailable: reason,
    item: const PlayingItemRequest(),
    artist: 'Artist',
    album: 'Album',
    title: 'Track',
    duration: 1,
  ),
);

void main() {
  const reasons = {
    PlaylistUnavailableReason.accessRevoked: 'Access was revoked.',
    PlaylistUnavailableReason.libraryUnavailable: 'Library is unavailable.',
    PlaylistUnavailableReason.sourceUnreachable: 'Source is unreachable.',
    PlaylistUnavailableReason.unreadable: 'Audio cannot be read.',
  };

  Future<void> pumpItem(
    WidgetTester tester,
    PlaylistUnavailableReason? reason, {
    VoidCallback onPlay = _noop,
  }) async {
    await tester.pumpWidget(
      ChangeNotifierProvider(
        create: (_) => RequestCapabilitiesProvider(),
        child: FluentApp(
          locale: const Locale('en'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: ScaffoldPage(
            content: PlaylistQueueItem(
              entry: queueEntry(reason),
              isCurrent: false,
              onPlay: onPlay,
            ),
          ),
        ),
      ),
    );
  }

  testWidgets('each unavailable reason is shown in the queue row', (
    tester,
  ) async {
    for (final entry in reasons.entries) {
      await pumpItem(tester, entry.key);
      expect(find.text(entry.value), findsOneWidget);
    }
  });

  testWidgets('an available item has no unavailable reason line', (
    tester,
  ) async {
    await pumpItem(tester, null);

    expect(find.byKey(const Key('queue-unavailable-reason')), findsNothing);
  });

  testWidgets('other-library queue rows render their supplied metadata', (
    tester,
  ) async {
    final entry = PlaylistEntry(
      0,
      PlaylistItem(
        item: const PlayingItemRequest(
          libraryItem: LibraryItemRequest(
            libraryId: 'library-uuid',
            fileUuid: 'file-uuid',
          ),
        ),
        artist: 'Other Artist',
        album: 'Other Album',
        title: 'Other Track',
        duration: 125,
      ),
    );
    await tester.pumpWidget(
      ChangeNotifierProvider(
        create: (_) => RequestCapabilitiesProvider(),
        child: FluentApp(
          locale: const Locale('en'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: ScaffoldPage(
            content: PlaylistQueueItem(
              entry: entry,
              isCurrent: false,
              onPlay: _noop,
            ),
          ),
        ),
      ),
    );

    expect(find.text('Other Track'), findsOneWidget);
    expect(find.text('Other Artist'), findsOneWidget);
    expect(entry.item.hasLocalFileId, isFalse);
  });

  testWidgets('tapping an unavailable item shows its reason without playing', (
    tester,
  ) async {
    var playRequests = 0;
    await pumpItem(
      tester,
      PlaylistUnavailableReason.accessRevoked,
      onPlay: () => playRequests += 1,
    );

    await tester.tap(find.text('Track'));
    await tester.pumpAndSettle();

    expect(
      find.descendant(
        of: find.byType(ContentDialog),
        matching: find.text('Access was revoked.'),
      ),
      findsOneWidget,
    );
    expect(playRequests, 0);
  });
}

void _noop() {}
