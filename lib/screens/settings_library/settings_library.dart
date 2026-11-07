import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/dialogs/failed_to_initialize_library.dart';
import '../../utils/l10n.dart';
import '../../utils/library_reference.dart';
import '../../utils/settings_page_padding.dart';
import '../../utils/settings_body_padding.dart';
import '../../utils/api/close_library.dart';
import '../../utils/router/navigation.dart';
import '../../utils/dialogs/select_library_mode/test_and_select_library_mode.dart';
import '../../widgets/library_task_button.dart';
import '../../widgets/settings/settings_button.dart';
import '../../widgets/settings/settings_tile_title.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../screens/settings_library/widgets/add_library_setting_button.dart';
import '../../providers/library_path.dart';
import '../../providers/library_manager.dart';
import '../../providers/broadcast.dart';
import '../../bindings/bindings.dart';
import '../../utils/api/library_registry.dart';
import '../../utils/library_ui/library_ui_state.dart';
import '../../utils/dialogs/information/error.dart';

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
    listLocalLibraries().then((response) {
      if (mounted) setState(() => registeredLibraries = response.libraries);
    }).catchError((Object error) {
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
      final response =
          await setLibraryBackgroundSync(library.libraryId, enabled);
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
    final libraryPath = Provider.of<LibraryPathProvider>(context, listen: true);
    final allOpenedFiles =
        libraryPath.getLLPaths().reversed.map((x) => x.rawPath).toList();
    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: false);

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

                      await closeLibrary(context);
                      libraryPath.clearAllOpenedFiles();
                      $$replace("/");
                    },
                  ),
                  const SizedBox(height: 2),
                  SizedBox(
                    width: double.maxFinite,
                    child: ListView.builder(
                      shrinkWrap: true,
                      itemCount: allOpenedFiles.length,
                      itemBuilder: (context, index) {
                        final itemPath = allOpenedFiles[index];
                        final aliases = {
                          for (final user
                              in Provider.of<BroadcastProvider>(context).users)
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
                            itemPath == libraryPath.currentPath;
                        final isSelectedLibrary = itemPath == selectedItem;

                        final scanProgress =
                            libraryManager.getScanTaskProgress(itemPath);
                        final analyzeProgress =
                            libraryManager.getAnalyzeTaskProgress(itemPath);

                        final scanWorking =
                            scanProgress?.status == TaskStatus.working;
                        final analyzeWorking =
                            analyzeProgress?.status == TaskStatus.working;

                        final initializing =
                            (scanProgress?.initialize ?? false) ||
                                (analyzeProgress?.isInitializeTask ?? false);

                        String fileName = libraryFolderName(itemPath);

                        void Function()? whileWorking(void Function() x) {
                          return isCurrentLibrary ||
                                  (initializing &&
                                      (scanWorking || analyzeWorking))
                              ? null
                              : x;
                        }

                        return ListTile.selectable(
                          title: SettingsTileTitle(
                            icon: shared ? Symbols.cloud_sync : Symbols.folder,
                            title: shared ? row.library!.alias : fileName,
                            subtitle: row.subtitle,
                            showActions: isSelectedLibrary,
                            actionsBuilder: (context) => Row(
                              children: [
                                Button(
                                  onPressed: whileWorking(
                                    () async {
                                      final result =
                                          await testAndSelectLibraryMode(
                                        context,
                                        allOpenedFiles[index],
                                      );

                                      if (result == null) return;
                                      final (initialized, initializeMode) =
                                          result;
                                      if (!initialized &&
                                          initializeMode == null) {
                                        return;
                                      }

                                      if (!context.mounted) return;
                                      await closeLibrary(context);

                                      if (!context.mounted) return;

                                      final (success, notReady, error, _) =
                                          await libraryPath.setLibraryPath(
                                        context,
                                        allOpenedFiles[index],
                                        initializeMode,
                                      );

                                      if (!success) {
                                        await showFailedToInitializeLibrary(
                                          context,
                                          error,
                                        );

                                        $$replace("/");
                                        return;
                                      }

                                      if (!context.mounted) return;
                                      $push('/library');
                                    },
                                  ),
                                  child: Text(S.of(context).switchTo),
                                ),
                                const SizedBox(width: 12),
                                Button(
                                  onPressed: whileWorking(() async {
                                    libraryPath.removeOpenedFile(
                                      allOpenedFiles[index],
                                    );
                                  }),
                                  child: Text(S.of(context).removeLibrary),
                                ),
                                if (row.library case final library?) ...[
                                  const SizedBox(width: 12),
                                  ToggleSwitch(
                                    content: Text(
                                      S.of(context).backgroundLibrarySync,
                                    ),
                                    checked: library.backgroundSync,
                                    onChanged: updatingLibraries
                                            .contains(library.libraryId)
                                        ? null
                                        : (value) =>
                                            _setBackgroundSync(library, value),
                                  ),
                                ],
                                if (isCurrentLibrary) ...[
                                  const SizedBox(width: 12),
                                  const ScanLibraryButton(),
                                  const SizedBox(width: 12),
                                  const AnalyzeLibraryButton(),
                                  const SizedBox(width: 12),
                                  const DeduplicateLibraryButton(),
                                ]
                              ],
                            ),
                          ),
                          selected: isSelectedLibrary,
                          onSelectionChange: (v) =>
                              setState(() => selectedItem = itemPath),
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
