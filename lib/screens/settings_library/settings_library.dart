import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/apple_music_sign_in.dart';
import '../../utils/dialogs/failed_to_initialize_library.dart';
import '../../utils/library_sign_in.dart';
import '../../utils/l10n.dart';
import '../../utils/settings_page_padding.dart';
import '../../utils/settings_body_padding.dart';
import '../../utils/router/navigation.dart';
import '../../widgets/library_task_button.dart';
import '../../widgets/settings/settings_button.dart';
import '../../widgets/settings/settings_box_toggle.dart';
import '../../widgets/settings/settings_tile_title.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../screens/settings_library/widgets/add_library_setting_button.dart';
import '../../providers/library_registry.dart';
import '../../providers/library_manager.dart';
import '../../providers/broadcast.dart';
import '../../bindings/bindings.dart';
import '../../utils/api/library_registry.dart';
import '../../utils/api/get_sync_settings.dart';
import '../../utils/api/set_sync_settings.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/share_sync_messages.dart';
import '../../utils/library_ui/library_ui_state.dart';
import '../../utils/dialogs/information/error.dart';

import 'utils/show_online_lyrics_dialog.dart';
import 'utils/show_reset_library_dialog.dart';
import 'widgets/share_sync_button.dart';

class SettingsLibraryPage extends StatefulWidget {
  const SettingsLibraryPage({
    super.key,
    this.loadLibraries,
    this.loadSyncSettings = getSyncSettings,
    this.saveSyncSettings = setSyncSettings,
  });

  final Future<ListLocalLibrariesResponse> Function()? loadLibraries;
  final Future<GetSyncSettingsResponse> Function() loadSyncSettings;
  final Future<void> Function(SyncSettingsChange change) saveSyncSettings;

  @override
  State<SettingsLibraryPage> createState() => _SettingsLibraryPageState();
}

class _SettingsLibraryPageState extends State<SettingsLibraryPage> {
  String selectedItem = '';

  List<LocalLibrary> registeredLibraries = [];
  GetSyncSettingsResponse? syncSettings;
  bool syncSettingsRequested = false;
  StreamSubscription? _contentSubscription;

  @override
  void initState() {
    super.initState();
    _loadLibraries();
    _contentSubscription = LibraryContentChanged.rustSignalStream.listen((_) {
      _loadLibraries();
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        Provider.of<BroadcastProvider>(context, listen: false).fetchUsers();
      }
    });
  }

  void _loadLibraries() {
    (widget.loadLibraries?.call() ?? listLocalLibraries())
        .then((response) {
          if (mounted) setState(() => registeredLibraries = response.libraries);
        })
        .catchError((Object error) {
          debugPrint('Could not load libraries: $error');
        });
  }

  @override
  void dispose() {
    _contentSubscription?.cancel();
    super.dispose();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    // The sync settings are only shown beside a library stored in a shared
    // folder, so they are loaded when the first such library is listed.
    final hasShareLibrary = Provider.of<LibraryRegistryProvider>(
      context,
    ).records.any((record) => record.shareSync);
    if (hasShareLibrary && !syncSettingsRequested) {
      syncSettingsRequested = true;
      _loadSyncSettings();
    }
  }

  Future<void> _loadSyncSettings() async {
    try {
      final settings = await widget.loadSyncSettings();
      if (mounted) setState(() => syncSettings = settings);
    } catch (error) {
      if (!mounted) return;
      await showErrorDialog(
        context: context,
        title: S.of(context).autoShareSync,
        errorMessage: requestErrorText(S.of(context), error),
      );
    }
  }

  Future<void> _setAutoShareSync(bool enabled) async {
    try {
      // The other settings are changed on the Devices page, so only this one
      // is sent and only this one is replaced in what is shown.
      final change = SyncSettingsChange(autoShareSync: enabled);
      await widget.saveSyncSettings(change);
      final shown = syncSettings;
      if (mounted && shown != null) {
        setState(() => syncSettings = change.applyTo(shown));
      }
    } catch (error) {
      if (!mounted) return;
      await showErrorDialog(
        context: context,
        title: S.of(context).autoShareSync,
        errorMessage: requestErrorText(S.of(context), error),
      );
    }
  }

  /// Syncs a library with its shared folder and reports a failure here: the
  /// hub announces a failed sync it was asked for to nobody else.
  Future<void> _syncShareLibrary(String libraryId) async {
    final libraryManager = Provider.of<LibraryManagerProvider>(
      context,
      listen: false,
    );
    String? message;
    try {
      final failure = await libraryManager.syncShareLibrary(libraryId);
      if (!mounted || failure == null) return;
      message = shareSyncFailureMessage(S.of(context), failure);
    } catch (error) {
      if (!mounted) return;
      message = requestErrorText(S.of(context), error);
    }
    await showErrorDialog(
      context: context,
      title: S.of(context).syncNow,
      errorMessage: message,
    );
  }

  @override
  Widget build(BuildContext context) {
    final registry = Provider.of<LibraryRegistryProvider>(context);
    final allRecords = registry.records.reversed
        .where(
          (record) => switch (record.kind) {
            LibraryRecordKindSummaryLocal() ||
            LibraryRecordKindSummaryReplica() ||
            LibraryRecordKindSummarySubsonic() ||
            LibraryRecordKindSummaryOpenDal() ||
            LibraryRecordKindSummaryAppleMusic() => true,
            _ => false,
          },
        )
        .toList();
    final invalidRecords = registry.invalidRecords.reversed.toList();
    final libraryManager = Provider.of<LibraryManagerProvider>(
      context,
      listen: false,
    );

    return PageContentFrame(
      builder: (context) => UnavailablePageOnBand(
        child: SettingsPagePadding(
          child: SingleChildScrollView(
            padding: getScrollContainerPadding(context),
            child: SettingsBodyPadding(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const AddLibrarySettingButton(
                    tryClose: true,
                    navigateIfFailed: true,
                  ),
                  SettingsButton(
                    icon: Symbols.refresh,
                    title: S.of(context).factoryReset,
                    subtitle: S.of(context).factoryResetSubtitle,
                    onPressed: () async {
                      final result = await showResetLibraryDialog(context);

                      if (!context.mounted) return;
                      if (result != true) return;

                      for (final record in registry.records.toList()) {
                        await registry.remove(
                          record.recordId,
                          deleteDeviceData: true,
                        );
                      }
                      $$replace("/");
                    },
                  ),
                  if (syncSettings case final settings?
                      when allRecords.any((record) => record.shareSync))
                    SettingsBoxToggle(
                      title: S.of(context).autoShareSync,
                      subtitle: S.of(context).autoShareSyncDescription,
                      value: settings.autoShareSync,
                      onChanged: _setAutoShareSync,
                    ),
                  const SizedBox(height: 2),
                  SizedBox(
                    width: double.maxFinite,
                    child: ListView.builder(
                      shrinkWrap: true,
                      itemCount: allRecords.length + invalidRecords.length,
                      itemBuilder: (context, index) {
                        if (index >= allRecords.length) {
                          final record =
                              invalidRecords[index - allRecords.length];
                          final isSelected = record.recordId == selectedItem;
                          final icon = switch (record.kind) {
                            LibraryRecordKindSummaryLocal() => Symbols.folder,
                            LibraryRecordKindSummaryReplica() =>
                              Symbols.cloud_sync,
                            LibraryRecordKindSummaryRemoteControl() =>
                              Symbols.devices,
                            LibraryRecordKindSummarySubsonic() =>
                              Symbols.folder,
                            _ => Symbols.folder,
                          };
                          return ListTile.selectable(
                            title: SettingsTileTitle(
                              icon: icon,
                              badgeContent: Icon(
                                Symbols.block,
                                size: 12,
                                color: Colors.white,
                              ),
                              badgeColor: Colors.red,
                              title: record.alias,
                              subtitle: S
                                  .of(context)
                                  .invalidLibrarySettingsSubtitle,
                              showActions: isSelected,
                              actionsBuilder: (context) => Wrap(
                                spacing: 12,
                                runSpacing: 8,
                                children: [
                                  Button(
                                    onPressed: () async {
                                      try {
                                        await registry.remove(record.recordId);
                                      } catch (error) {
                                        if (!context.mounted) return;
                                        await showErrorDialog(
                                          context: context,
                                          title: S.of(context).unknownError,
                                          errorMessage: libraryFailureMessage(
                                            S.of(context),
                                            error,
                                          ),
                                        );
                                      }
                                    },
                                    child: Text(S.of(context).removeLibrary),
                                  ),
                                ],
                              ),
                            ),
                            onSelectionChange: (selected) => setState(
                              () => selectedItem = selected
                                  ? record.recordId
                                  : '',
                            ),
                          );
                        }
                        final record = allRecords[index];
                        final itemPath = switch (record.kind) {
                          LibraryRecordKindSummaryLocal(:final path) => path,
                          _ => '',
                        };
                        final aliases = {
                          for (final user in Provider.of<BroadcastProvider>(
                            context,
                          ).users)
                            if (user.status == ClientStatus.approved)
                              user.fingerprint: user.alias,
                        };
                        final row = switch (record.kind) {
                          // A Subsonic library keeps no catalog on this
                          // device, so no local library belongs to its row.
                          LibraryRecordKindSummarySubsonic(:final endpoint) =>
                            LibraryRowState(null, endpoint),
                          // A WebDAV or S3 library has no folder on this
                          // device, so its local library cannot be found by
                          // path.
                          LibraryRecordKindSummaryOpenDal(:final endpoint) =>
                            LibraryRowState(null, endpoint),
                          // An Apple Music library keeps no catalog on this
                          // device either. Its row says where it plays.
                          LibraryRecordKindSummaryAppleMusic() =>
                            LibraryRowState(
                              null,
                              appleMusicSubtitleText(S.of(context)),
                            ),
                          _ => libraryRowState(
                            itemPath,
                            registeredLibraries,
                            aliases,
                            S.of(context).libraryFromDevice,
                            S.of(context).readOnly,
                            describePath: (path) =>
                                describeLibraryPath(S.of(context), path),
                          ),
                        };
                        final owner = row.library?.ownerFingerprint;
                        final shared = owner != null && owner.isNotEmpty;
                        final isCurrentLibrary =
                            record.recordId == registry.openRecordId;
                        final isSelectedLibrary =
                            record.recordId == selectedItem;

                        final scanProgress = libraryManager.getScanTaskProgress(
                          record.recordId,
                        );
                        final analyzeProgress = libraryManager
                            .getAnalyzeTaskProgress(record.recordId);

                        final scanWorking =
                            scanProgress?.status == TaskStatus.working;
                        final analyzeWorking =
                            analyzeProgress?.status == TaskStatus.working;

                        final initializing =
                            (scanProgress?.initialize ?? false) ||
                            (analyzeProgress?.isInitializeTask ?? false);

                        String fileName = switch (record.kind) {
                          LibraryRecordKindSummaryReplica() ||
                          LibraryRecordKindSummarySubsonic() ||
                          LibraryRecordKindSummaryOpenDal() ||
                          LibraryRecordKindSummaryAppleMusic() => record.alias,
                          _ => localFolderName(itemPath),
                        };

                        void Function()? whileWorking(void Function() x) {
                          return isCurrentLibrary ||
                                  (initializing &&
                                      (scanWorking || analyzeWorking))
                              ? null
                              : x;
                        }

                        final subtitle = row.subtitle;

                        return ListTile.selectable(
                          title: SettingsTileTitle(
                            icon: switch (record.kind) {
                              LibraryRecordKindSummarySubsonic() ||
                              LibraryRecordKindSummaryAppleMusic() =>
                                Symbols.music_note,
                              LibraryRecordKindSummaryOpenDal() =>
                                Symbols.cloud,
                              _ when shared => Symbols.cloud_sync,
                              _ => Symbols.folder,
                            },
                            title: shared ? row.library!.alias : fileName,
                            subtitle: subtitle,
                            showActions: isSelectedLibrary,
                            actionsBuilder: (context) => Wrap(
                              spacing: 12,
                              runSpacing: 8,
                              children: [
                                Button(
                                  onPressed: whileWorking(() async {
                                    try {
                                      // A sign-in the user was told about
                                      // and closed leaves them here.
                                      if (!await openLibraryWithSignIn(
                                        context,
                                        registry,
                                        record.recordId,
                                      )) {
                                        return;
                                      }
                                    } catch (error) {
                                      if (!context.mounted) return;
                                      await showFailedToInitializeLibrary(
                                        context,
                                        libraryFailureMessage(
                                          S.of(context),
                                          error,
                                        ),
                                      );
                                      $$replace('/');
                                      return;
                                    }

                                    if (!context.mounted) return;
                                    $push('/library');
                                  }),
                                  child: Text(S.of(context).switchTo),
                                ),
                                // A library whose lyrics come from online
                                // says whether the user allowed it to look
                                // them up, which the user changes here.
                                if (record.onlineLyrics
                                    case final onlineLyrics?)
                                  Button(
                                    onPressed: () => showOnlineLyricsDialog(
                                      context,
                                      enabled: onlineLyrics,
                                      onChanged: (enabled) =>
                                          registry.setOnlineLyrics(
                                            record.recordId,
                                            enabled,
                                          ),
                                    ),
                                    child: Text(S.of(context).lyrics),
                                  ),
                                Button(
                                  onPressed:
                                      (initializing &&
                                          (scanWorking || analyzeWorking))
                                      ? null
                                      : () async {
                                          // Removing the open library closes it, so leave the
                                          // library pages as Factory Reset does.
                                          final wasOpen = isCurrentLibrary;
                                          try {
                                            await registry.remove(
                                              record.recordId,
                                            );
                                            if (wasOpen) $$replace("/");
                                          } catch (error) {
                                            if (!context.mounted) return;
                                            await showErrorDialog(
                                              context: context,
                                              title: S.of(context).unknownError,
                                              errorMessage:
                                                  libraryFailureMessage(
                                                    S.of(context),
                                                    error,
                                                  ),
                                            );
                                          }
                                        },
                                  child: Text(S.of(context).removeLibrary),
                                ),
                                if (record case LibraryRecordSummary(
                                  shareSync: true,
                                  kind: LibraryRecordKindSummaryOpenDal(
                                    :final libraryId,
                                  ),
                                ))
                                  ShareSyncButton(
                                    libraryId: libraryId,
                                    onPressed: () =>
                                        _syncShareLibrary(libraryId),
                                  ),
                                if (isCurrentLibrary) ...[
                                  const ScanLibraryButton(),
                                  const AnalyzeLibraryButton(),
                                  const DeduplicateLibraryButton(),
                                ],
                              ],
                            ),
                          ),
                          selected: isSelectedLibrary,
                          onSelectionChange: (v) =>
                              setState(() => selectedItem = record.recordId),
                        );
                      },
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
