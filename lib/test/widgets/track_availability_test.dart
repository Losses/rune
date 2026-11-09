import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../widgets/track_list/large_screen_track_list_item.dart';
import '../../utils/query_list.dart';

InternalMediaFile item(
  AudioAvailability availability, [
  AudioOrigin origin = AudioOrigin.host,
]) => InternalMediaFile(
  id: 1,
  path: '',
  artist: 'Artist',
  album: 'Album',
  title: 'Track',
  duration: 1,
  coverArtPath: '',
  trackNumber: 1,
  audioAvailability: availability,
  audioOrigin: origin,
);

void main() {
  testWidgets(
    'local state controls labels and pending playback without hiding rows',
    (tester) async {
      await tester.pumpWidget(
        FluentApp(
          locale: const Locale('en'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: Builder(
            builder: (context) {
              final rows = AudioAvailability.values.map(item).toList();
              return Column(
                children: [
                  for (final row in rows)
                    Text(
                      '${row.audioAvailability.name}: ${row.canPlay}: ${row.availabilityLabel(context) ?? "local"}',
                    ),
                ],
              );
            },
          ),
        ),
      );
      expect(find.text('local: true: local'), findsOneWidget);
      expect(
        find.text('remoteOnly: true: Remote Audio'),
        findsOneWidget,
      );
      expect(
        find.text('unavailable: false: Audio Unavailable'),
        findsOneWidget,
      );
      expect(
        find.text('importPending: false: Importing'),
        findsOneWidget,
      );
    },
  );
  testWidgets('synced copies show their origin next to the artist', (
    tester,
  ) async {
    await tester.pumpWidget(
      FluentApp(
        locale: const Locale('en'),
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Builder(
          builder: (context) => Column(
            children: [
              Text(
                item(
                  AudioAvailability.local,
                  AudioOrigin.syncedCopy,
                ).artistWithAvailability(context),
              ),
              Text(
                item(
                  AudioAvailability.local,
                  AudioOrigin.host,
                ).artistWithAvailability(context),
              ),
              Text(
                item(
                  AudioAvailability.remoteOnly,
                  AudioOrigin.remote,
                ).artistWithAvailability(context),
              ),
            ],
          ),
        ),
      ),
    );
    expect(find.text('Artist • Synced Copy'), findsOneWidget);
    expect(find.text('Artist'), findsOneWidget);
    expect(
      find.text('Artist • Remote Audio'),
      findsOneWidget,
    );
  });
  testWidgets('pending rows stay visible but cannot start playback', (
    tester,
  ) async {
    Future<void> render(AudioAvailability state) async {
      await tester.pumpWidget(
        FluentApp(
          locale: const Locale('en'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: SizedBox(
            width: 400,
            height: 80,
            child: LargeScreenTrackListItem(
              index: 0,
              item: item(state),
              queries: const QueryList(),
              mode: 0,
              fallbackFileIds: const [],
              coverArtPath: null,
              isAlbumQuery: true,
              position: 0,
              reloadData: null,
            ),
          ),
        ),
      );
      await tester.pump();
    }

    await render(AudioAvailability.importPending);
    expect(find.textContaining('Importing'), findsOneWidget);
    expect(tester.widget<Button>(find.byType(Button).first).onPressed, isNull);
    await render(AudioAvailability.unavailable);
    expect(find.textContaining('Audio Unavailable'), findsOneWidget);
    expect(tester.widget<Button>(find.byType(Button).first).onPressed, isNull);
    await render(AudioAvailability.remoteOnly);
    expect(
      find.textContaining('Remote Audio'),
      findsOneWidget,
    );
    expect(
      tester.widget<Button>(find.byType(Button).first).onPressed,
      isNotNull,
    );
  });
}
