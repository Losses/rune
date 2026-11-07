import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/playlist.dart';
import '../../providers/renderer_attachment.dart';
import '../../providers/library_path.dart';
import '../../providers/status.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/dialogs/play_on/play_on_dialog.dart';
import '../../utils/dialogs/play_queue_dialog.dart';
import '../../utils/playing_item.dart';
import '../../utils/execute_middle_click_action.dart';

void main() {
  testWidgets('unavailable reasons are dimmed and cannot be selected', (
    tester,
  ) async {
    var attached = 0;
    final candidates = [
      _candidate('offline', RendererAvailability.offline),
      _candidate('unapproved', RendererAvailability.notApproved),
      _candidate('no-control', RendererAvailability.noControl),
      _candidate(
        'cant-play',
        RendererAvailability.cantPlay,
        blockedLibraryId: 'blocked-library',
      ),
    ];
    await _open(
      tester,
      candidates: candidates,
      handoff: (_) async => attached++,
    );

    expect(find.text('Offline'), findsOneWidget);
    expect(find.text('This device has not approved you yet'), findsOneWidget);
    expect(find.text("Can't control this device"), findsOneWidget);
    expect(find.text("Can't play My Music"), findsOneWidget);
    for (final alias in ['offline', 'unapproved', 'no-control', 'cant-play']) {
      final row = tester.widget<ListTile>(
        find.ancestor(of: find.text(alias), matching: find.byType(ListTile)),
      );
      expect(row.onPressed, isNull);
      await tester.tap(find.text(alias));
    }
    expect(attached, 0);
  });

  testWidgets(
    'available renderer hands off and closes; current target is marked',
    (tester) async {
      String? handoffTarget;
      final renderer = AttachedRenderer(fingerprint: 'fp-b', alias: 'Bedroom');
      await _open(
        tester,
        candidates: [
          RendererCandidate(
            fingerprint: renderer.fingerprint,
            alias: renderer.alias,
            attached: true,
            availability: RendererAvailability.available,
          ),
          _candidate(
            'Kitchen',
            RendererAvailability.available,
            fingerprint: 'fp-k',
          ),
        ],
        initial: RendererAttachmentChanged(renderer: renderer),
        handoff: (target) async => handoffTarget = target,
      );

      expect(find.text('Bedroom'), findsOneWidget);
      expect(find.byIcon(Symbols.check), findsOneWidget);
      await tester.tap(find.text('Kitchen'));
      await tester.pumpAndSettle();
      expect(handoffTarget, 'fp-k');
      expect(find.text('Kitchen'), findsNothing);
    },
  );

  testWidgets('this device hands off back and loader failures can be retried', (
    tester,
  ) async {
    var handedOffToThisDevice = false;
    var loads = 0;
    await _open(
      tester,
      initial: RendererAttachmentChanged(
        renderer: const AttachedRenderer(fingerprint: 'fp-b', alias: 'Bedroom'),
      ),
      loader: (_) async {
        loads++;
        if (loads == 1) {
          throw const RequestFailure(
            'ListRenderersRequest',
            RequestErrorKind.unavailable,
            'offline',
          );
        }
        return [
          _candidate(
            'Bedroom',
            RendererAvailability.available,
            fingerprint: 'fp-b',
            attached: true,
          ),
        ];
      },
      handoff: (target) async => handedOffToThisDevice = target == null,
    );
    expect(find.text("This isn't available right now"), findsOneWidget);
    await tester.tap(find.text('Retry'));
    await tester.pumpAndSettle();
    expect(loads, 2);
    await tester.tap(find.text('This device'));
    await tester.pumpAndSettle();
    expect(handedOffToThisDevice, isTrue);
    expect(find.text('Play on'), findsNothing);
  });

  test(
    'queue library ids are distinct, ordered, and skip non-library items',
    () {
      expect(
        queueLibraryIds([
          const PlayingItem.libraryItem(libraryId: 'a', fileUuid: '1'),
          const PlayingItem.independentFile('/file'),
          const PlayingItem.libraryItem(libraryId: 'b', fileUuid: '2'),
          const PlayingItem.libraryItem(libraryId: 'a', fileUuid: '3'),
          const PlayingItem.inLibrary(4),
        ]),
        ['a', 'b'],
      );
    },
  );

  testWidgets('cast mode omits this device and can cast to an attached row', (
    tester,
  ) async {
    List<String>? requestedLibraries;
    CastTarget? castTarget;
    await _open(
      tester,
      candidates: [
        _candidate(
          'Bedroom',
          RendererAvailability.available,
          fingerprint: 'fp-b',
          attached: true,
        ),
      ],
      loader: (ids) async {
        requestedLibraries = ids;
        return [
          _candidate(
            'Bedroom',
            RendererAvailability.available,
            fingerprint: 'fp-b',
            attached: true,
          ),
        ];
      },
      castLibraryIds: const ['library-h'],
      cast: (candidate) async {
        castTarget = castTargetFor(candidate.fingerprint, true);
      },
    );

    expect(find.text('This device'), findsNothing);
    expect(requestedLibraries, ['library-h']);
    await tester.tap(find.text('Bedroom'));
    await tester.pumpAndSettle();
    expect(castTarget, const CastTarget(fingerprint: 'fp-b', attach: true));
    expect(find.text('Play on'), findsNothing);
  });

  testWidgets('typed cast failures name the renderer and blocked library', (
    tester,
  ) async {
    await _open(
      tester,
      candidates: [_candidate('Bedroom', RendererAvailability.available)],
      castLibraryIds: const ['blocked-library'],
      cast: (_) async {
        throw const RequestFailure(
          'OperatePlaybackWithMixQueryRequest',
          RequestErrorKind.permissionDenied,
          'denied',
          RequestFailureCauseCastNotPossible(
            libraryId: 'blocked-library',
            state: CastLibraryState.notHeld,
          ),
        );
      },
    );

    await tester.tap(find.text('Bedroom'));
    await tester.pumpAndSettle();
    expect(find.text("Bedroom can't play My Music"), findsOneWidget);
    expect(find.text('Play on'), findsOneWidget);
  });

  testWidgets('cast failures without a cause keep the generic message', (
    tester,
  ) async {
    await _open(
      tester,
      candidates: [_candidate('Bedroom', RendererAvailability.available)],
      castLibraryIds: const ['blocked-library'],
      cast: (_) async {
        throw const RequestFailure(
          'OperatePlaybackWithMixQueryRequest',
          RequestErrorKind.permissionDenied,
          'denied',
        );
      },
    );

    await tester.tap(find.text('Bedroom'));
    await tester.pumpAndSettle();
    expect(find.text("This device isn't allowed to do that"), findsOneWidget);
  });

  testWidgets('handoff reports typed cast failures in the dialog', (
    tester,
  ) async {
    await _open(
      tester,
      candidates: [_candidate('Bedroom', RendererAvailability.available)],
      handoff: (_) async {
        throw const RequestFailure(
          'HandoffRequest',
          RequestErrorKind.permissionDenied,
          'denied',
          RequestFailureCauseCastNotPossible(
            libraryId: 'blocked-library',
            state: CastLibraryState.notHeld,
          ),
        );
      },
    );

    await tester.tap(find.text('Bedroom'));
    await tester.pumpAndSettle();
    expect(find.text("Bedroom can't play My Music"), findsOneWidget);
    expect(find.text('Play on'), findsOneWidget);
  });

  testWidgets('queue panel opens Play on for the open library', (tester) async {
    var opened = 0;
    await _openQueue(
      tester,
      libraryPath: '/music',
      onPlayOn: (_) async => opened++,
    );
    await tester.tap(find.byKey(const Key('queue-play-on')));
    await tester.pumpAndSettle();
    expect(opened, 1);
    await tester.tap(find.byIcon(Symbols.close));
    await tester.pumpAndSettle();
  });
}

RendererCandidate _candidate(
  String alias,
  RendererAvailability availability, {
  String? blockedLibraryId,
  String? fingerprint,
  bool attached = false,
}) => RendererCandidate(
  fingerprint: fingerprint ?? alias,
  alias: alias,
  attached: attached,
  availability: availability,
  blockedLibraryId: blockedLibraryId,
  libraryState: availability == RendererAvailability.cantPlay
      ? CastLibraryState.notHeld
      : null,
);

Future<void> _open(
  WidgetTester tester, {
  List<RendererCandidate> candidates = const [],
  RendererLoader? loader,
  RendererHandoff? handoff,
  RendererAttachmentChanged? initial,
  List<String>? castLibraryIds,
  RendererCast? cast,
}) async {
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider(create: (_) => PlaylistProvider()),
        ChangeNotifierProvider(
          create: (_) => RendererAttachmentProvider(initial: initial),
        ),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Builder(
          builder: (context) => Button(
            child: const Text('open'),
            onPressed: () => showPlayOnDialog(
              context,
              loader: loader ?? (_) async => candidates,
              handoff: handoff ?? (_) async {},
              loadLibraryNames: () async => {'blocked-library': 'My Music'},
              castLibraryIds: castLibraryIds,
              cast: cast,
            ),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('open'));
  await tester.pumpAndSettle();
}

Future<void> _openQueue(
  WidgetTester tester, {
  required String? libraryPath,
  required PlayOnDialogOpener onPlayOn,
}) async {
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider(create: (_) => PlaylistProvider()),
        ChangeNotifierProvider(create: (_) => PlaybackStatusProvider()),
        ChangeNotifierProvider<LibraryPathProvider>(
          create: (_) => _TestLibraryPathProvider(libraryPath),
        ),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Builder(
          builder: (context) => Button(
            child: const Text('open queue'),
            onPressed: () =>
                showPlayQueueDialog(context, playOnDialog: onPlayOn),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('open queue'));
  await tester.pumpAndSettle();
}

class _TestLibraryPathProvider extends LibraryPathProvider {
  _TestLibraryPathProvider(this.path) : super(null);

  final String? path;

  @override
  String? get currentPath => path;
}
