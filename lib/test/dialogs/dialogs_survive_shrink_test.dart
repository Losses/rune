import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:provider/single_child_widget.dart';
import 'package:rune/bindings/bindings.dart' hide WalSelfTestResult;
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
import 'package:rune/screens/collection/utils/collection_data_provider.dart';
import 'package:rune/screens/settings_about/settings_about.dart';
import 'package:rune/screens/settings_laboratory/widgets/settings/fs_self_test_settings.dart';
import 'package:rune/screens/settings_neighbors/utils/show_add_remote_device_dialog.dart';
import 'package:rune/screens/settings_neighbors/utils/show_edit_device_information_setting_dialog.dart';
import 'package:rune/screens/settings_neighbors/utils/show_fingerprint_quiz_dialog.dart';
import 'package:rune/screens/settings_neighbors/utils/show_server_status_dialog.dart';
import 'package:rune/screens/settings_neighbors/widgets/search_remote_device_dialog.dart';
import 'package:rune/screens/settings_theme/widgets/visualizer_scenes_setting.dart';
import 'package:rune/utils/api/run_fs_self_test.dart';
import 'package:rune/utils/context_menu/offline_selection.dart';
import 'package:rune/utils/dialogs/show_group_list_dialog.dart';
import 'package:rune/widgets/start_screen/utils/group.dart';
import 'package:rune/widgets/start_screen/utils/internal_collection.dart';
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

/// Pumps [home] at desktop size with the responsive provider the shrink tests
/// move through the small layouts, and returns that provider.
Future<_Responsive> _pumpHarness(
  WidgetTester tester, {
  required Widget home,
  List<SingleChildWidget> providers = const [],
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
        home: home,
      ),
    ),
  );
  return responsive;
}

/// Types into the first field, shrinks the window to each small layout and
/// back, and checks that nothing overflows, [controls] can still be reached,
/// what was typed and [retainedTexts] are still there, and [atLayout] holds.
Future<void> _shrinkAndRestore(
  WidgetTester tester,
  _Responsive responsive, {
  bool hasField = true,
  List<Finder> controls = const [],
  List<String> retainedTexts = const [],
  Future<void> Function(WidgetTester tester, String layout)? atLayout,
  Future<void> Function(WidgetTester tester)? afterRestore,
}) async {
  if (hasField) {
    await tester.enterText(find.byType(EditableText).first, 'typed text');
    await tester.pump();
  }
  for (final (name, size, device) in _layouts) {
    tester.view.physicalSize = size;
    responsive.change(device);
    await tester.pump();
    expect(tester.takeException(), isNull, reason: 'overflow in the $name');
    for (final control in controls) {
      await tester.ensureVisible(control);
      await tester.pump();
      expect(
        control.hitTestable(),
        findsOneWidget,
        reason: '$control is not reachable in the $name',
      );
    }
    for (final text in retainedTexts) {
      expect(
        find.text(text),
        findsWidgets,
        reason: '"$text" is gone in the $name',
      );
    }
    await atLayout?.call(tester, name);
  }
  tester.view.physicalSize = const Size(1280, 800);
  responsive.change(DeviceType.tv);
  await tester.pump();
  expect(tester.takeException(), isNull);
  if (hasField) {
    expect(find.text('typed text'), findsOneWidget);
  }
  for (final text in retainedTexts) {
    expect(find.text(text), findsWidgets);
  }
  await afterRestore?.call(tester);
}

/// Opens a dialog at desktop size with [open], then runs [_shrinkAndRestore].
Future<void> _survivesShrinking(
  WidgetTester tester,
  _Open open, {
  bool hasField = true,
  bool settle = true,
  List<SingleChildWidget> providers = const [],
  Future<void> Function(WidgetTester tester)? afterOpen,
  List<Finder> controls = const [],
  List<String> retainedTexts = const [],
  Future<void> Function(WidgetTester tester, String layout)? atLayout,
  Future<void> Function(WidgetTester tester)? afterRestore,
  TransitionBuilder? appBuilder,
}) async {
  final responsive = await _pumpHarness(
    tester,
    home: Builder(
      builder: (context) =>
          Button(child: const Text('open'), onPressed: () => open(context)),
    ),
    providers: providers,
    appBuilder: appBuilder,
  );
  await tester.tap(find.text('open'));
  if (settle) {
    await tester.pumpAndSettle();
  } else {
    // A dialog that animates while it waits never settles.
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 500));
  }
  expect(tester.takeException(), isNull);
  await afterOpen?.call(tester);
  await _shrinkAndRestore(
    tester,
    responsive,
    hasField: hasField,
    controls: controls,
    retainedTexts: retainedTexts,
    atLayout: atLayout,
    afterRestore: afterRestore,
  );
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
            shareSync: false,
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

  testWidgets('the add remote device dialog survives the window shrinking', (
    tester,
  ) async {
    await _survivesShrinking(
      tester,
      (context) => showAddRemoteDeviceDialog(false, context),
      controls: [
        find.widgetWithText(FilledButton, 'Add Connection'),
        find.widgetWithText(Button, 'Cancel'),
      ],
      retainedTexts: const ['7863'],
      afterRestore: (tester) async {
        await tester.tap(find.widgetWithText(Button, 'Cancel'));
        await tester.pumpAndSettle();
        expect(find.text('Add Connection'), findsNothing);
      },
    );
  });

  testWidgets(
    'the edit device information dialog survives the window shrinking',
    (tester) async {
      final broadcast = _Broadcast();
      addTearDown(broadcast.dispose);
      await _survivesShrinking(
        tester,
        (context) => showEditDeviceInformationSettingDialog(context),
        providers: [
          ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
        ],
        controls: [
          find.widgetWithText(FilledButton, 'Edit'),
          find.widgetWithText(Button, 'Cancel'),
        ],
        afterRestore: (tester) async {
          await tester.tap(find.widgetWithText(FilledButton, 'Edit'));
          await tester.pump();
          expect(broadcast.deviceAlias, 'typed text');
          await tester.tap(find.widgetWithText(Button, 'Cancel'));
          await tester.pumpAndSettle();
          expect(find.text('Edit device information'), findsNothing);
        },
      );
    },
  );

  testWidgets('the fingerprint quiz dialog survives the window shrinking', (
    tester,
  ) async {
    await _survivesShrinking(
      tester,
      (context) => showFingerprintQuizDialog(
        context,
        '192.0.2.1:7863',
        loadFingerprint: (_) async => 'A1B2C3D4E5F60718293A4B5C6D7E8F90',
      ),
      hasField: false,
      controls: [find.widgetWithText(Button, 'Cancel')],
      retainedTexts: const [
        'Pairing',
        'For security verification, select the characters that complete the '
            "fingerprint shown on your partner device. This confirms you're "
            'connecting to the right device.',
      ],
      afterRestore: (tester) async {
        await tester.tap(find.widgetWithText(Button, 'Cancel'));
        await tester.pumpAndSettle();
        expect(find.text('Pairing'), findsNothing);
      },
    );
  });

  testWidgets('the server status dialog survives the window shrinking', (
    tester,
  ) async {
    var checks = 0;
    var approved = false;
    await _survivesShrinking(
      tester,
      (context) => showServerStatusDialog(
        context,
        const ['192.0.2.1:7863'],
        'peer-fingerprint',
        check: (_, _) async {
          checks++;
          return CheckDeviceOnServerResponse(
            status: approved ? ClientStatus.approved : ClientStatus.pending,
          );
        },
      ),
      hasField: false,
      settle: false,
      controls: [find.widgetWithText(Button, 'Cancel')],
      retainedTexts: const [
        'Waiting for the server to approve this request...',
      ],
      afterRestore: (tester) async {
        final before = checks;
        approved = true;
        await tester.pump(const Duration(seconds: 2));
        await tester.pumpAndSettle();
        expect(checks, greaterThan(before));
        expect(
          find.text('Waiting for the server to approve this request...'),
          findsNothing,
        );
      },
    );
  });

  testWidgets('the search remote device dialog survives the window shrinking', (
    tester,
  ) async {
    DiscoveredDeviceMessage? selected;
    await _survivesShrinking(
      tester,
      (context) => $showModal<DiscoveredDeviceMessage>(
        context,
        (context, close) => SearchRemoteDeviceDialog(
          $close: (device) {
            selected = device;
            close(device);
          },
          initialDevices: const [
            DiscoveredDeviceMessage(
              alias: 'Bedroom',
              deviceModel: 'MacBook Pro',
              deviceType: DeviceTypeMessage.desktop,
              fingerprint: 'peer-fingerprint',
              lastSeenUnixEpoch: 0,
              addresses: ['192.0.2.1:7863'],
            ),
            DiscoveredDeviceMessage(
              alias: 'Living Room',
              deviceModel: 'Rune Player',
              deviceType: DeviceTypeMessage.headless,
              fingerprint: 'living-room-fingerprint',
              lastSeenUnixEpoch: 0,
              addresses: ['192.0.2.2:7863'],
            ),
          ],
        ),
      ),
      hasField: false,
      settle: false,
      providers: [
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
        ),
      ],
      controls: [
        find.widgetWithText(Button, 'Pair'),
        find.widgetWithText(Button, 'Cancel'),
      ],
      retainedTexts: const ['Bedroom', 'Living Room'],
      afterOpen: (tester) async {
        await tester.tap(find.text('Bedroom'));
        await tester.pump();
      },
      afterRestore: (tester) async {
        await tester.tap(find.widgetWithText(Button, 'Pair'));
        await tester.pumpAndSettle();
        expect(selected?.alias, 'Bedroom');
      },
    );
  });

  testWidgets('the filesystem self-test dialog survives the window shrinking', (
    tester,
  ) async {
    const layers = [
      FsSelfTestLayerResult(
        layer: 'Dart file I/O',
        outcome: FsSelfTestLayerOutcome.passed,
        elapsedMs: 12,
        detail: 'Wrote and read a 1 MiB file.',
      ),
      FsSelfTestLayerResult(
        layer: 'Rust file I/O',
        outcome: FsSelfTestLayerOutcome.passed,
        elapsedMs: 34,
        detail: 'Wrote and read a 1 MiB file through the Rust backend.',
      ),
      FsSelfTestLayerResult(
        layer: 'SAF file I/O',
        outcome: FsSelfTestLayerOutcome.skipped,
        elapsedMs: 0,
        detail: 'Not available on this platform.',
      ),
    ];
    Future<FsSelfTestResult> run(
      String path, {
      void Function(FsSelfTestProgress progress)? onProgress,
    }) async {
      for (final layer in layers) {
        onProgress?.call(
          FsSelfTestProgress(
            layer: layer.layer,
            outcome: layer.outcome,
            elapsedMs: layer.elapsedMs,
            detail: layer.detail,
          ),
        );
      }
      return const FsSelfTestResult(
        outcome: FsSelfTestOutcome.passed,
        layers: layers,
        walProbe: WalSelfTestResult(supported: true, detail: 'WAL works.'),
      );
    }

    await _survivesShrinking(
      tester,
      (context) => $showModal<void>(
        context,
        (context, close) => FsSelfTestDialog(
          path: '/storage/emulated/0/Music',
          $close: close,
          run: run,
        ),
      ),
      hasField: false,
      controls: [find.widgetWithText(Button, 'Close')],
      retainedTexts: const [
        'Filesystem self-test',
        'All filesystem layers passed.',
        'Dart file I/O · 12 ms',
      ],
      afterRestore: (tester) async {
        await tester.tap(find.text('Dart file I/O · 12 ms'));
        await tester.pumpAndSettle();
        expect(find.text('Wrote and read a 1 MiB file.'), findsOneWidget);
        await tester.tap(find.widgetWithText(Button, 'Close'));
        await tester.pumpAndSettle();
        expect(find.text('Filesystem self-test'), findsNothing);
      },
    );
  });

  testWidgets('the group list dialog survives the window shrinking', (
    tester,
  ) async {
    CollectionCache().clearType(CollectionType.album);
    addTearDown(() => CollectionCache().clearType(CollectionType.album));
    CollectionCache().cacheSummary(CollectionType.album, [
      for (final title in ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H'])
        Group<InternalCollection>(groupTitle: title, items: const []),
    ]);
    final data = CollectionDataProvider(collectionType: CollectionType.album);
    addTearDown(data.dispose);
    String? jumpedTo;
    await _survivesShrinking(
      tester,
      (context) => showGroupListDialog(context, (title) async {
        jumpedTo = title;
      }),
      hasField: false,
      providers: [
        ChangeNotifierProvider<CollectionDataProvider>.value(value: data),
      ],
      controls: [
        find.widgetWithText(Button, 'A'),
        find.widgetWithText(Button, 'Cancel'),
      ],
      retainedTexts: const ['A', 'B', 'C'],
      afterRestore: (tester) async {
        await tester.tap(find.widgetWithText(Button, 'C'));
        await tester.pumpAndSettle();
        expect(jumpedTo, 'C');
      },
    );
  });

  testWidgets('the offline cleanup dialog survives the window shrinking', (
    tester,
  ) async {
    bool? result;
    await _survivesShrinking(
      tester,
      (context) => $showModal<bool>(
        context,
        (context, close) => OfflineCleanupDialog(
          initialValue: true,
          onClose: (value) {
            result = value;
            close(value);
          },
        ),
      ),
      hasField: false,
      controls: [
        find.widgetWithText(FilledButton, 'Confirm'),
        find.widgetWithText(Button, 'Cancel'),
      ],
      retainedTexts: const ['Delete Unneeded Downloads'],
      afterOpen: (tester) async {
        await tester.tap(find.byType(Checkbox));
        await tester.pump();
      },
      atLayout: (tester, layout) async {
        expect(
          tester.widget<Checkbox>(find.byType(Checkbox)).checked,
          isFalse,
          reason: 'the checkbox lost its selection in the $layout',
        );
      },
      afterRestore: (tester) async {
        expect(tester.widget<Checkbox>(find.byType(Checkbox)).checked, isFalse);
        await tester.tap(find.widgetWithText(FilledButton, 'Confirm'));
        await tester.pumpAndSettle();
        expect(result, isFalse);
      },
    );
  });

  testWidgets('the scene delete confirmation survives the window shrinking', (
    tester,
  ) async {
    await _survivesShrinking(
      tester,
      (context) => showDeleteSceneDialog(context, 'Aurora'),
      hasField: false,
      controls: [
        find.widgetWithText(FilledButton, 'Delete'),
        find.widgetWithText(Button, 'Cancel'),
      ],
      retainedTexts: const [
        'Delete Aurora? Import its scene file again to get it back.',
      ],
      afterRestore: (tester) async {
        await tester.tap(find.widgetWithText(Button, 'Cancel'));
        await tester.pumpAndSettle();
        expect(find.textContaining('Delete Aurora?'), findsNothing);
      },
    );
  });

  testWidgets('the about dialog survives the window shrinking', (tester) async {
    await _survivesShrinking(
      tester,
      showMysteriousDialog,
      hasField: false,
      controls: [find.widgetWithText(Button, 'Close')],
      retainedTexts: const [
        'Creak',
        "It seems you've triggered a mysterious mechanism. The room's "
            'structure appears to have changed, revealing new areas to '
            'explore.',
      ],
      afterRestore: (tester) async {
        await tester.tap(find.widgetWithText(Button, 'Close'));
        await tester.pumpAndSettle();
        expect(find.text('Creak'), findsNothing);
      },
    );
  });
}

class _LocalLibrary extends ChangeNotifier implements LibraryOpenStateProvider {
  @override
  bool get isRemoteControl => false;

  @override
  LibraryRecordSummary? get openRecord => const LibraryRecordSummary(
    shareSync: false,
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
  String? deviceAlias = 'Office';

  @override
  List<ClientSummary> get users => const [];

  @override
  String? get fingerprint => 'fingerprint';

  @override
  void fetchUsers() {}

  @override
  Future<void> updateDeviceAlias(String newAlias) async {
    deviceAlias = newAlias;
    notifyListeners();
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _EmptyRegistry extends ChangeNotifier implements LibraryRegistryProvider {
  @override
  List<LibraryRecordSummary> get records => const [];

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
