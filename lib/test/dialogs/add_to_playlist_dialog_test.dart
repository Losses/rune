import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/responsive_providers.dart';
import '../../screens/settings_library/widgets/progress_button.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/dialogs/add_to_playlist/add_to_playlist_dialog.dart';

const playlistId = 3;

/// A library whose album 1 has the songs 11, 12 and 13, and the playlist
/// [playlistId] that they are added to, one at a time as the test lets them.
class Library {
  Library({List<int> inPlaylist = const [], this.failsAt, this.timesOutAt});

  final List<int> playlist = [];
  late final _initial = playlist.length;

  /// The song whose add the library refuses, once.
  int? failsAt;

  /// The song whose add gets no answer in time, once.
  int? timesOutAt;
  var reads = 0;
  var closed = 0;
  final _pending = <Completer<void>>[];

  Future<List<int>> fileIds(CollectionType type, int id) async {
    reads++;
    return type == CollectionType.playlist
        ? List.of(playlist)
        : const [11, 12, 13];
  }

  Future<void> add(int playlistId, int fileId) async {
    final turn = Completer<void>();
    _pending.add(turn);
    await turn.future;
    if (fileId == failsAt) {
      failsAt = null;
      throw const RequestFailure(
        'AddItemToPlaylistRequest',
        RequestErrorKind.unavailable,
        'refused',
      );
    }
    if (fileId == timesOutAt) {
      timesOutAt = null;
      throw TimeoutException('no answer');
    }
    playlist.add(fileId);
  }

  /// The songs added since the dialog opened.
  List<int> get added => playlist.sublist(_initial);

  /// Lets the add that waits go through.
  Future<void> next(WidgetTester tester) async {
    _pending.removeAt(0).complete();
    await tester.pump();
    await tester.pump();
  }
}

Future<Library> pumpDialog(
  WidgetTester tester, {
  List<int> inPlaylist = const [],
  int? failsAt,
  int? timesOutAt,
}) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = const Size(1920, 1080);
  addTearDown(tester.view.reset);
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final library = Library(failsAt: failsAt, timesOutAt: timesOutAt)..playlist.addAll(inPlaylist);
  // Reads the count the playlist starts with.
  library.added;
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
        ChangeNotifierProvider<ResponsiveProvider>(
          create: (_) => ResponsiveProvider(sizes, () async => null),
        ),
      ],
      child: FluentApp(
        locale: const Locale('en'),
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        // The test font is about twice as wide as the app's, which would not
        // leave the button room for its text.
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(
            context,
          ).copyWith(textScaler: const TextScaler.linear(0.5)),
          child: child!,
        ),
        home: AddToPlaylistDialog(
          type: CollectionType.album,
          id: 1,
          playlistId: playlistId,
          loadFileIds: library.fileIds,
          addFileToPlaylist: library.add,
          $close: (_) => library.closed++,
        ),
      ),
    ),
  );
  await tester.pump();
  await tester.pump();
  return library;
}

AddToPlaylistDialogState stateOf(WidgetTester tester) =>
    tester.state<AddToPlaylistDialogState>(find.byType(AddToPlaylistDialog));

/// The ring of the progress button, as the share it shows.
double? ring(WidgetTester tester) =>
    tester.widget<ProgressRing>(find.byType(ProgressRing)).value;

void main() {
  testWidgets('shows how many songs are added, and closes when all are', (
    tester,
  ) async {
    final library = await pumpDialog(tester);

    expect(find.text('Adding to Playlist'), findsOneWidget);
    expect(find.text('0 of 3'), findsOneWidget);
    expect(ring(tester), 0);

    await library.next(tester);
    expect(find.text('1 of 3'), findsOneWidget);
    expect(ring(tester), closeTo(100 / 3, 0.01));
    expect(library.closed, 0);

    await library.next(tester);
    await library.next(tester);
    expect(library.added, [11, 12, 13]);
    expect(library.closed, 1);
    await tester.pump(const Duration(seconds: 1));
  });

  testWidgets('cannot be started again while it runs', (tester) async {
    final library = await pumpDialog(tester);

    // The button that would start it shows the progress and takes no press.
    expect(find.byType(FilledButton), findsNothing);
    expect(
      tester.widget<ProgressButton>(find.byType(ProgressButton)).onPressed,
      isNull,
    );
    final reads = library.reads;
    await stateOf(tester).start();
    await tester.pump();
    expect(library.reads, reads);

    await library.next(tester);
    await library.next(tester);
    await library.next(tester);
    expect(library.added, [11, 12, 13]);
    await tester.pump(const Duration(seconds: 1));
  });

  testWidgets('leaves out the songs the playlist already has', (tester) async {
    final library = await pumpDialog(tester, inPlaylist: [12]);

    expect(find.text('0 of 2'), findsOneWidget);
    await library.next(tester);
    await library.next(tester);

    expect(library.added, [11, 13]);
    expect(library.closed, 1);
    await tester.pump(const Duration(seconds: 1));
  });

  testWidgets('closes at once when the playlist has every song', (
    tester,
  ) async {
    final library = await pumpDialog(tester, inPlaylist: [13, 12, 11]);

    expect(library.added, isEmpty);
    expect(library.closed, 1);
    await tester.pump(const Duration(seconds: 1));
  });

  testWidgets('stops at a failure and reports it', (tester) async {
    final library = await pumpDialog(tester, failsAt: 12);

    await library.next(tester);
    await library.next(tester);

    expect(library.added, [11]);
    expect(library.closed, 0);
    expect(stateOf(tester).failure, isNotNull);
    expect(find.byType(ProgressButton), findsNothing);
    final message = requestFailureMessage(
      AppLocalizations.of(tester.element(find.byType(ContentDialog)))!,
      stateOf(tester).failure!,
    );
    expect(find.text(message), findsOneWidget);

    // Starting again adds only what is still missing.
    await tester.tap(find.byType(FilledButton));
    await tester.pump();
    await tester.pump();
    expect(find.text('0 of 2'), findsOneWidget);
    await library.next(tester);
    await library.next(tester);
    expect(library.added, [11, 12, 13]);
    expect(library.closed, 1);
    await tester.pump(const Duration(seconds: 1));
  });

  testWidgets('stops at a request that gets no answer and can start again', (
    tester,
  ) async {
    final library = await pumpDialog(tester, timesOutAt: 12);

    await library.next(tester);
    await library.next(tester);

    expect(library.added, [11]);
    expect(stateOf(tester).isLoading, false);
    expect(stateOf(tester).failure, isNotNull);
    expect(find.byType(ProgressButton), findsNothing);
    final s = AppLocalizations.of(tester.element(find.byType(ContentDialog)))!;
    expect(find.text(s.unknownError), findsOneWidget);
    expect(find.byType(FilledButton), findsOneWidget);
    await tester.pump(const Duration(seconds: 1));
  });

  testWidgets('adds no more once it is closed', (tester) async {
    final library = await pumpDialog(tester);

    await library.next(tester);
    await tester.pumpWidget(const SizedBox.shrink());
    await library.next(tester);

    expect(library.added, [11, 12]);
    expect(library._pending, isEmpty);
    await tester.pump(const Duration(seconds: 1));
  });
}
