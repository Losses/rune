import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/playlist.dart';
import '../../providers/renderer_attachment.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/library_open_state.dart';
import '../../providers/status.dart';
import '../../providers/responsive_providers.dart';
import '../../widgets/settings/settings_tile_title.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/dialogs/play_on/play_on_dialog.dart';
import '../../utils/dialogs/play_queue_dialog.dart';
import '../../utils/playing_item.dart';
import '../../utils/execute_middle_click_action.dart';

void main() {
  testWidgets('unavailable reasons disable the selected row action', (
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
    expect(find.text('This device has not approved access.'), findsOneWidget);
    expect(find.text("This device cannot be controlled."), findsOneWidget);
    expect(find.text("Cannot play My Music."), findsOneWidget);
    await tester.tap(find.text('offline'));
    await tester.pumpAndSettle();
    final playButton = tester.widget<Button>(
      find
          .ancestor(of: find.text('Play On'), matching: find.byType(Button))
          .first,
    );
    expect(playButton.onPressed, isNull);
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
      expect(find.byIcon(Symbols.play_arrow), findsOneWidget);
      await tester.tap(find.text('Kitchen'));
      await tester.pumpAndSettle();
      await _tapPlayAction(tester);
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
    expect(find.text("This is not available right now."), findsOneWidget);
    await tester.tap(find.text('Retry'));
    await tester.pumpAndSettle();
    expect(loads, 2);
    await tester.tap(find.text('This device'));
    await tester.pumpAndSettle();
    await _tapPlayAction(tester);
    await tester.pumpAndSettle();
    expect(handedOffToThisDevice, isTrue);
    expect(find.text('Play On'), findsNothing);
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
    await _tapPlayAction(tester);
    await tester.pumpAndSettle();
    expect(castTarget, const CastTarget(fingerprint: 'fp-b', attach: true));
    expect(find.text('Play On'), findsNothing);
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
    await _tapPlayAction(tester);
    await tester.pumpAndSettle();
    expect(find.text("Bedroom cannot play My Music."), findsOneWidget);
    expect(find.text('Play On'), findsNWidgets(2));
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
    await _tapPlayAction(tester);
    await tester.pumpAndSettle();
    expect(find.text("This device is not allowed to do that."), findsOneWidget);
  });

  testWidgets(
    'selected renderer actions and load errors stay in ContentDialog',
    (tester) async {
      await _open(
        tester,
        candidates: [_candidate('Bedroom', RendererAvailability.available)],
      );
      await tester.tap(find.text('Bedroom'));
      await tester.pumpAndSettle();
      expect(find.byType(SettingsTileTitle), findsNWidgets(2));
      expect(find.text('Play On'), findsNWidgets(2));
      expect(find.text('Close'), findsOneWidget);

      await tester.pumpWidget(const SizedBox.shrink());
      await _open(
        tester,
        loader: (_) async => throw const RequestFailure(
          'ListRenderersRequest',
          RequestErrorKind.unavailable,
          'offline',
        ),
      );
      expect(find.byType(ContentDialog), findsOneWidget);
      expect(find.text('This is not available right now.'), findsOneWidget);
      expect(find.text('Retry'), findsOneWidget);
      expect(find.byType(InfoBar), findsNothing);
    },
  );

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
    await _tapPlayAction(tester);
    await tester.pumpAndSettle();
    expect(find.text("Bedroom cannot play My Music."), findsOneWidget);
    expect(find.text('Play On'), findsNWidgets(2));
  });

  testWidgets('queue panel opens Play on and hides it for a remote library', (
    tester,
  ) async {
    var opened = 0;
    await _openQueue(tester, remote: false, onPlayOn: (_) async => opened++);
    await tester.tap(find.byKey(const Key('queue-play-on')));
    await tester.pumpAndSettle();
    expect(opened, 1);
    await tester.tap(find.byIcon(Symbols.close));
    await tester.pumpAndSettle();

    await tester.pumpWidget(const SizedBox.shrink());
    await _openQueue(tester, remote: true, onPlayOn: (_) async => opened++);
    expect(find.byKey(const Key('queue-play-on')), findsNothing);
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
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
        ),
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
  required bool remote,
  required PlayOnDialogOpener onPlayOn,
}) async {
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider(create: (_) => PlaylistProvider()),
        ChangeNotifierProvider(create: (_) => PlaybackStatusProvider()),
        ChangeNotifierProvider(create: (_) => RequestCapabilitiesProvider()),
        ChangeNotifierProvider<LibraryOpenStateProvider>(
          create: (_) => _TestLibraryOpenStateProvider(remote: remote),
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

Future<void> _tapPlayAction(WidgetTester tester) async {
  await tester.tap(
    find
        .ancestor(of: find.text('Play On').last, matching: find.byType(Button))
        .first,
  );
}

class _TestLibraryOpenStateProvider extends ChangeNotifier
    implements LibraryOpenStateProvider {
  _TestLibraryOpenStateProvider({required this.remote});

  final bool remote;

  @override
  bool get isRemoteControl => remote;

  @override
  LibraryRecordSummary? get openRecord => remote
      ? const LibraryRecordSummary(
          recordId: 'remote-record',
          alias: 'Renderer',
          kind: LibraryRecordKindSummaryRemoteControl(fingerprint: 'renderer'),
        )
      : const LibraryRecordSummary(
          recordId: 'local-record',
          alias: 'Music',
          kind: LibraryRecordKindSummaryLocal(
            libraryId: 'library-id',
            path: '/music',
          ),
        );

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
