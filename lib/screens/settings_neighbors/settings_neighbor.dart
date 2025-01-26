import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/settings_page_padding.dart';
import '../../utils/settings_body_padding.dart';
import '../../utils/api/close_library.dart';
import '../../utils/router/navigation.dart';
import '../../utils/dialogs/select_library_mode/test_and_select_library_mode.dart';
import '../../widgets/library_task_button.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../screens/settings_library/widgets/add_library_setting_button.dart';
import '../../providers/library_path.dart';
import '../../providers/library_manager.dart';
import '../../utils/l10n.dart';

import 'widgets/manually_add_remote_device_setting_button.dart';
import 'widgets/settings_tile_title.dart';

class SettingsLibraryPage extends StatefulWidget {
  const SettingsLibraryPage({super.key});

  @override
  State<SettingsLibraryPage> createState() => _SettingsLibraryPageState();
}

class _SettingsLibraryPageState extends State<SettingsLibraryPage> {
  String selectedItem = '';

  bool requested = false;
  List<String> allOpenedFiles = [];

  @override
  void didChangeDependencies() {
    if (requested) return;

    final libraryPath =
        Provider.of<LibraryPathProvider>(context, listen: false);
    libraryPath.getAllOpenedFiles().then((x) {
      if (!context.mounted) return;

      setState(() {
        allOpenedFiles = x.reversed.toList();
      });
    });

    super.didChangeDependencies();
  }

  @override
  Widget build(BuildContext context) {
    final libraryPath = Provider.of<LibraryPathProvider>(context, listen: true);
    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: false);

    return PageContentFrame(
      child: UnavailablePageOnBand(
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
                  const ManuallyAddRemoteDeviceSettingButton(
                    tryClose: true,
                    navigateIfFailed: true,
                  ),
                  const SizedBox(height: 2),
                  SizedBox(
                    width: double.maxFinite,
                    child: ListView.builder(
                      shrinkWrap: true,
                      itemCount: allOpenedFiles.length,
                      itemBuilder: (context, index) {
                        final itemPath = allOpenedFiles[index];
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

                        String fileName = File(itemPath).uri.pathSegments.last;

                        void Function()? whileWorking(void Function() x) {
                          return isCurrentLibrary ||
                                  (initializing &&
                                      (scanWorking || analyzeWorking))
                              ? null
                              : x;
                        }

                        return ListTile.selectable(
                          title: SettingsTileTitle(
                            icon: Symbols.folder,
                            title: fileName,
                            subtitle: allOpenedFiles[index],
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

                                      libraryPath.setLibraryPath(
                                        context,
                                        allOpenedFiles[index],
                                        initializeMode,
                                      );

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
                                if (isCurrentLibrary) ...[
                                  const SizedBox(width: 12),
                                  const ScanLibraryButton(),
                                  const SizedBox(width: 12),
                                  const AnalyzeLibraryButton(),
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
