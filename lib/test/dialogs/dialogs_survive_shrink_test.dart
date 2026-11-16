import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:provider/single_child_widget.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/broadcast.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/request_capabilities.dart';
import 'package:rune/providers/status.dart';
import 'package:rune/providers/playlist.dart';
import 'package:rune/providers/renderer_attachment.dart';
import 'package:rune/screens/settings_log/widgets/log_detail_dialog.dart';
import 'package:rune/screens/settings_server/utils/show_review_connection_dialog.dart';
import 'package:rune/screens/settings_server/widgets/peer_libraries_dialog.dart';
import 'package:rune/utils/dialogs/add_library/add_library_guide.dart';
import 'package:rune/utils/dialogs/play_on/play_on_dialog.dart';
import 'package:rune/utils/dialogs/play_queue_dialog.dart';
import 'package:rune/utils/dialogs/subsonic/subsonic_relogin_dialog.dart';
import 'package:rune/utils/router/navigation.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_filter.dart';
import 'package:rune/widgets/visualizer/lab/lab_series.dart';
import 'package:rune/widgets/visualizer/lab/lab_views.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/settings_library/utils/show_reset_library_dialog.dart';
import 'package:rune/utils/dialogs/information/confirm.dart';
import 'package:rune/utils/dialogs/information/error.dart';
import 'package:rune/utils/dialogs/information/information.dart';
import 'package:rune/utils/dialogs/select_library_mode/show_select_library_mode_dialog.dart';
import 'package:rune/widgets/library_task_button.dart';
import 'package:rune/utils/dialogs/register/show_register_dialog.dart';
import 'package:rune/utils/dialogs/scrobble/show_scrobble_login_dialog.dart';

class _Responsive extends ChangeNotifier implements ResponsiveProvider {
  DeviceType device = DeviceType.tv;

  void change(DeviceType value) {
    device = value;
    notifyListeners();
  }

  @override
  DeviceType getActiveDeviceType(List<DeviceType> deviceTypes) => device;

  @override
  bool smallerOrEqualTo(DeviceType type, [bool autoOrientation = true]) =>
      false;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

/// The layouts a window can shrink to while a dialog is open.
const _layouts = <(String, Size, DeviceType)>[
  ('dock', Size(100, 800), DeviceType.dock),
  ('band', Size(1280, 120), DeviceType.band),
  ('belt', Size(1280, 200), DeviceType.belt),
];

typedef _Open = void Function(BuildContext context);

/// Opens a dialog at desktop size, types into its first field, shrinks the
/// window to each small layout and back, and checks that nothing overflows and
/// what was typed is still there.
Future<void> _survivesShrinking(
  WidgetTester tester,
  _Open open, {
  bool hasField = true,
  List<SingleChildWidget> providers = const [],
  Future<void> Function(WidgetTester tester)? afterOpen,
  TransitionBuilder? appBuilder,
}) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = const Size(1280, 800);
  addTearDown(tester.view.reset);
  final responsive = _Responsive();
  addTearDown(responsive.dispose);
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
        ...providers,
      ],
      child: FluentApp(
        builder: appBuilder,
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Builder(
          builder: (context) =>
              Button(child: const Text('open'), onPressed: () => open(context)),
        ),
      ),
    ),
  );
  await tester.tap(find.text('open'));
  await tester.pumpAndSettle();
  expect(tester.takeException(), isNull);
  await afterOpen?.call(tester);

  if (hasField) {
    await tester.enterText(find.byType(EditableText).first, 'typed text');
    await tester.pump();
  }
  for (final (name, size, device) in _layouts) {
    tester.view.physicalSize = size;
    responsive.change(device);
    await tester.pump();
    expect(tester.takeException(), isNull, reason: 'overflow in the $name');
  }
  tester.view.physicalSize = const Size(1280, 800);
  responsive.change(DeviceType.tv);
  await tester.pump();
  expect(tester.takeException(), isNull);
  if (hasField) {
    expect(find.text('typed text'), findsOneWidget);
  }
}

void main() {
  final dialogs = <String, (_Open, bool)>{
    'information': (
      (context) => showInformationDialog(
        context: context,
        title: 'Title',
        subtitle: 'A subtitle that is long enough to wrap in a small window.',
      ),
      false,
    ),
    'confirm': (
      (context) => showConfirmDialog(
        context: context,
        title: 'Title',
        subtitle: 'A subtitle that is long enough to wrap in a small window.',
        yesLabel: 'Yes',
        noLabel: 'No',
      ),
      false,
    ),
    'error': (
      (context) => showErrorDialog(
        context: context,
        title: 'Title',
        errorMessage: 'Something went wrong in a way that needs some words.',
      ),
      false,
    ),
    'reset library': ((context) => showResetLibraryDialog(context), false),
    'select library mode': (
      (context) => showSelectLibraryModeDialog(context),
      false,
    ),
    'library task': ((context) => showCancelDialog(context), false),
    'scrobble login': (
      (context) => showScrobbleLoginDialog(context, 'lastfm', 'Last.fm'),
      true,
    ),
    'register': ((context) => showRegisterDialog(context), false),
  };
  for (final entry in dialogs.entries) {
    testWidgets('the ${entry.key} dialog survives the window shrinking', (
      tester,
    ) async {
      await _survivesShrinking(
        tester,
        entry.value.$1,
        hasField: entry.value.$2,
      );
    });
  }

  testWidgets('the play on dialog survives the window shrinking', (
    tester,
  ) async {
    await _survivesShrinking(
      tester,
      (context) => showPlayOnDialog(
        context,
        loader: (_) async => [
          const RendererCandidate(
            fingerprint: 'fp',
            alias: 'Bedroom',
            attached: false,
            availability: RendererAvailability.available,
          ),
        ],
        handoff: (_) async {},
        loadLibraryNames: () async => {},
      ),
      hasField: false,
      providers: [
        ChangeNotifierProvider(create: (_) => PlaylistProvider()),
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
        ),
        ChangeNotifierProvider(create: (_) => RendererAttachmentProvider()),
      ],
    );
  });

  testWidgets('the peer libraries dialog survives the window shrinking', (
    tester,
  ) async {
    await _survivesShrinking(
      tester,
      (context) => $showModal<void>(
        context,
        (context, close) => PeerLibrariesDialog(
          client: const ClientSummary(
            alias: 'Bedroom',
            fingerprint: 'peer-fingerprint',
            deviceModel: 'Desktop',
            status: ClientStatus.approved,
          ),
          close: () => close(null),
          loadLibraries: (_) async => const ListPeerLibrariesResponse(
            libraries: [
              PeerLibrary(
                libraryId: 'shared-library',
                alias: 'Shared Music',
                added: false,
                connection: DeviceConnectionLevel.view,
                sync: DeviceSyncLevel.pull,
              ),
            ],
          ),
          addLibrary: (_, _) async =>
              const AddRemoteLibraryResponse(recordId: ''),
        ),
      ),
      hasField: false,
      providers: [
        ChangeNotifierProvider<LibraryRegistryProvider>.value(
          value: _EmptyRegistry(),
        ),
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
        ),
      ],
    );
  });

  testWidgets('the add-library guide survives the window shrinking', (
    tester,
  ) async {
    final registry = LibraryRegistryProvider(
      loadRecords: () async => ListLibraryRecordsResponse(
        records: const [],
        invalidRecords: const [],
        lastOpenRecord: null,
      ),
    );
    addTearDown(registry.dispose);
    final manager = LibraryManagerProvider(sendScanRequest: (_) {});
    addTearDown(manager.dispose);
    final broadcast = _Broadcast();
    addTearDown(broadcast.dispose);
    await _survivesShrinking(
      tester,
      (context) => showAddLibraryGuide(context),
      hasField: false,
      providers: [
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
        ),
        ChangeNotifierProvider<LibraryRegistryProvider>.value(value: registry),
        ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
        ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
      ],
    );
  });

  testWidgets('the add-library guide form survives the window shrinking', (
    tester,
  ) async {
    final registry = LibraryRegistryProvider(
      loadRecords: () async => ListLibraryRecordsResponse(
        records: const [],
        invalidRecords: const [],
        lastOpenRecord: null,
      ),
    );
    addTearDown(registry.dispose);
    final manager = LibraryManagerProvider(sendScanRequest: (_) {});
    addTearDown(manager.dispose);
    final broadcast = _Broadcast();
    addTearDown(broadcast.dispose);
    await _survivesShrinking(
      tester,
      (context) => showAddLibraryGuide(context),
      afterOpen: (tester) async {
        await tester.tap(find.text('WebDAV'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Confirm'));
        await tester.pumpAndSettle();
      },
      providers: [
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
        ),
        ChangeNotifierProvider<LibraryRegistryProvider>.value(value: registry),
        ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
        ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
      ],
    );
  });

  testWidgets('the play queue dialog survives the window shrinking', (
    tester,
  ) async {
    await _survivesShrinking(
      tester,
      (context) => showPlayQueueDialog(context, playOnDialog: (_) async {}),
      hasField: false,
      providers: [
        ChangeNotifierProvider(create: (_) => PlaylistProvider()),
        ChangeNotifierProvider(create: (_) => PlaybackStatusProvider()),
        ChangeNotifierProvider(create: (_) => RequestCapabilitiesProvider()),
        ChangeNotifierProvider<LibraryOpenStateProvider>(
          create: (_) => _LocalLibrary(),
        ),
      ],
    );
  });

  testWidgets('the Subsonic sign-in dialog survives the window shrinking', (
    tester,
  ) async {
    final registry = LibraryRegistryProvider(
      loadRecords: () async => ListLibraryRecordsResponse(
        records: const [
          LibraryRecordSummary(
            recordId: 'subsonic-record',
            alias: 'Subsonic',
            kind: LibraryRecordKindSummarySubsonic(
              endpoint: 'https://music.example',
              account: '',
            ),
          ),
        ],
        invalidRecords: const [],
        lastOpenRecord: null,
      ),
    );
    addTearDown(registry.dispose);
    await registry.ready;
    await _survivesShrinking(
      tester,
      (context) =>
          showSubsonicReloginDialog(context, registry, 'subsonic-record'),
    );
  });

  testWidgets('the lab rows dialog survives the window shrinking', (
    tester,
  ) async {
    await _survivesShrinking(
      tester,
      (context) => showLabRowsDialog(
        context,
        model: LabRowsModel(),
        columns: const [
          labSectionsColumn,
          labMarksColumn,
          LabColumn(
            id: 'drive',
            name: 'Drive',
            group: 'Signals',
            kind: LabColumnKind.number,
          ),
        ],
      ),
      hasField: false,
      appBuilder: (context, child) => RevealEffectContext(child: child!),
      providers: [
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
        ),
      ],
    );
  });

  testWidgets('the log detail dialog survives the window shrinking', (
    tester,
  ) async {
    await _survivesShrinking(
      tester,
      (context) => $showModal<bool>(
        context,
        (context, close) => LogDetailDialog(
          logs: const [
            LogDetail(
              id: 1,
              level: LogDetailLevel.error,
              domain: 'scan',
              detail: 'A long detail of what went wrong while scanning.',
              date: 0,
            ),
          ],
          initialIndex: 0,
          onClose: () => close(false),
        ),
      ),
      hasField: false,
    );
  });

  testWidgets('the review connection dialog survives the window shrinking', (
    tester,
  ) async {
    final broadcast = _Broadcast();
    addTearDown(broadcast.dispose);
    await _survivesShrinking(
      tester,
      (context) => showReviewConnectionDialog(
        context,
        const ClientSummary(
          alias: 'Bedroom',
          fingerprint: 'peer-fingerprint',
          deviceModel: 'Desktop',
          status: ClientStatus.pending,
        ),
      ),
      hasField: false,
      providers: [
        ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
      ],
    );
  });
}

class _LocalLibrary extends ChangeNotifier implements LibraryOpenStateProvider {
  @override
  bool get isRemoteControl => false;

  @override
  LibraryRecordSummary? get openRecord => const LibraryRecordSummary(
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

class _Broadcast extends ChangeNotifier implements BroadcastProvider {
  @override
  List<ClientSummary> get users => const [];

  @override
  String? get fingerprint => 'fingerprint';

  @override
  void fetchUsers() {}

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _EmptyRegistry extends ChangeNotifier implements LibraryRegistryProvider {
  @override
  List<LibraryRecordSummary> get records => const [];

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
