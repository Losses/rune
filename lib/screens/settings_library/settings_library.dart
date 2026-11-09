import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/dialogs/failed_to_initialize_library.dart';
import '../../utils/l10n.dart';
import '../../utils/settings_page_padding.dart';
import '../../utils/settings_body_padding.dart';
import '../../utils/router/navigation.dart';
import '../../widgets/library_task_button.dart';
import '../../widgets/settings/settings_button.dart';
import '../../widgets/settings/settings_tile_title.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../screens/settings_library/widgets/add_library_setting_button.dart';
import '../../providers/library_registry.dart';
import '../../providers/library_manager.dart';
import '../../providers/broadcast.dart';
import '../../bindings/bindings.dart';
import '../../utils/api/library_registry.dart';
import '../../utils/library_ui/library_ui_state.dart';
import '../../utils/dialogs/information/error.dart';
import '../../utils/library_registry_messages.dart';

import 'utils/show_reset_library_dialog.dart';

class SettingsLibraryPage extends StatefulWidget {
  const SettingsLibraryPage({super.key});

  @override
  State<SettingsLibraryPage> createState() => _SettingsLibraryPageState();
}

class _SettingsLibraryPageState extends State<SettingsLibraryPage> {
  String selectedItem = '';

  List<LocalLibrary> registeredLibraries = [];
  final Set<String> updatingLibraries = {};
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
    listLocalLibraries()
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

  Future<void> _setBackgroundSync(LocalLibrary library, bool enabled) async {
    setState(() {
      updatingLibraries.add(library.libraryId);
      registeredLibraries = registeredLibraries
          .map(
            (entry) => entry.libraryId == library.libraryId
                ? entry.copyWith(backgroundSync: enabled)
                : entry,
          )
          .toList();
    });
    String? error;
    try {
      final response = await setLibraryBackgroundSync(
        library.libraryId,
        enabled,
      );
      error = response.error;
    } catch (exception) {
      error = exception.toString();
    }
    if (!mounted) return;
    setState(() {
      updatingLibraries.remove(library.libraryId);
      if (error != null) {
        registeredLibraries = registeredLibraries
            .map(
              (entry) => entry.libraryId == library.libraryId
                  ? entry.copyWith(backgroundSync: library.backgroundSync)
                  : entry,
            )
            .toList();
      }
    });
    if (error != null) {
      await showErrorDialog(
        context: context,
        title: S.of(context).unknownError,
        errorMessage: error,
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final registry = Provider.of<LibraryRegistryProvider>(context);
    final allRecords = registry.records.reversed
        .where(
          (record) => switch (record.kind) {
            LibraryRecordKindSummaryLocal() ||
            LibraryRecordKindSummaryReplica() => true,
            _ => false,
          },
        )
        .toList();
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
                        await registry.remove(record.recordId);
                      }
                      $$replace("/");
                    },
                  ),
                  const SizedBox(height: 2),
                  SizedBox(
                    width: double.maxFinite,
                    child: ListView.builder(
                      shrinkWrap: true,
                      itemCount: allRecords.length,
                      itemBuilder: (context, index) {
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
                        final row = libraryRowState(
                          itemPath,
                          registeredLibraries,
                          aliases,
                          S.of(context).libraryFromDevice,
                          S.of(context).readOnly,
                          describePath: (path) =>
                              describeLibraryPath(S.of(context), path),
                        );
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
                          LibraryRecordKindSummaryReplica() => record.alias,
                          _ => localFolderName(itemPath),
                        };

                        void Function()? whileWorking(void Function() x) {
                          return isCurrentLibrary ||
                                  (initializing &&
                                      (scanWorking || analyzeWorking))
                              ? null
                              : x;
                        }

                        final subtitle = switch (row.library) {
                          final library? when library.backgroundSync =>
                            '${row.subtitle} • ${S.of(context).backgroundLibrarySync}',
                          _ => row.subtitle,
                        };

                        return ListTile.selectable(
                          title: SettingsTileTitle(
                            icon: shared ? Symbols.cloud_sync : Symbols.folder,
                            title: shared ? row.library!.alias : fileName,
                            subtitle: subtitle,
                            showActions: isSelectedLibrary,
                            actionsBuilder: (context) => Row(
                              children: [
                                Button(
                                  onPressed: whileWorking(() async {
                                    try {
                                      await registry.open(record.recordId);
                                    } catch (error) {
                                      if (!context.mounted) return;
                                      await showFailedToInitializeLibrary(
                                        context,
                                        error is LibraryRegistryFailure
                                            ? libraryRegistryFailureMessage(
                                                S.of(context),
                                                error.kind,
                                              )
                                            : S.of(context).unknownError,
                                      );
                                      $$replace('/');
                                      return;
                                    }

                                    if (!context.mounted) return;
                                    $push('/library');
                                  }),
                                  child: Text(S.of(context).switchTo),
                                ),
                                const SizedBox(width: 12),
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
                                                  error
                                                      is LibraryRegistryFailure
                                                  ? libraryRegistryFailureMessage(
                                                      S.of(context),
                                                      error.kind,
                                                    )
                                                  : S.of(context).unknownError,
                                            );
                                          }
                                        },
                                  child: Text(S.of(context).removeLibrary),
                                ),
                                if (row.library case final library?) ...[
                                  const SizedBox(width: 12),
                                  Button(
                                    onPressed:
                                        updatingLibraries.contains(
                                          library.libraryId,
                                        )
                                        ? null
                                        : () => _setBackgroundSync(
                                            library,
                                            !library.backgroundSync,
                                          ),
                                    child: Text(
                                      library.backgroundSync
                                          ? S.of(context).turnOffBackgroundSync
                                          : S.of(context).turnOnBackgroundSync,
                                    ),
                                  ),
                                ],
                                if (isCurrentLibrary) ...[
                                  const SizedBox(width: 12),
                                  const ScanLibraryButton(),
                                  const SizedBox(width: 12),
                                  const AnalyzeLibraryButton(),
                                  const SizedBox(width: 12),
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
