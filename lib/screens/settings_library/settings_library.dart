import 'dart:io';

import 'package:provider/provider.dart';
import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:file_selector/file_selector.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/settings_page_padding.dart';
import '../../utils/settings_body_padding.dart';
import '../../utils/api/close_library.dart';
import '../../widgets/library_task_button.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../providers/library_path.dart';
import '../../providers/library_manager.dart';

import 'widgets/settings_button.dart';
import 'widgets/settings_tile_title.dart';

class SettingsLibraryPage extends StatefulWidget {
  const SettingsLibraryPage({super.key});

  @override
  State<SettingsLibraryPage> createState() => _SettingsLibraryPageState();
}

class _SettingsLibraryPageState extends State<SettingsLibraryPage> {
  String selectedItem = '';

  @override
  Widget build(BuildContext context) {
    final libraryPath = Provider.of<LibraryPathProvider>(context, listen: true);
    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: true);

    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SettingsPagePadding(
          child: SingleChildScrollView(
            child: SettingsBodyPadding(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SettingsButton(
                      icon: Symbols.add,
                      title: "Add Library",
                      subtitle: "Add a new library and scan existing files",
                      onPressed: () async {
                        final path = await getDirectoryPath();

                        if (path == null) return;

                        if (!context.mounted) return;
                        await closeLibrary(context);
                        libraryPath.setLibraryPath(path, true);
                        libraryManager.scanLibrary(path, false);

                        if (!context.mounted) return;
                        context.go('/library');
                      }),
                  SettingsButton(
                    icon: Symbols.refresh,
                    title: "Factory Reset",
                    subtitle: "Remove all items from the library list",
                    onPressed: () async {
                      await closeLibrary(context);

                      libraryPath.clearAllOpenedFiles();
                    },
                  ),
                  const SizedBox(height: 2),
                  FutureBuilder(
                    future: libraryPath.getAllOpenedFiles(),
                    builder: (context, snapshot) {
                      if (snapshot.connectionState == ConnectionState.waiting) {
                        return Container();
                      } else if (snapshot.hasError) {
                        return Center(child: Text('Error: ${snapshot.error}'));
                      }

                      final allOpenedFiles = snapshot.data!;

                      return SizedBox(
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
                            final analyseProgress =
                                libraryManager.getAnalyseTaskProgress(itemPath);

                            final scanWorking =
                                scanProgress?.status == TaskStatus.working;
                            final analyseWorking =
                                analyseProgress?.status == TaskStatus.working;

                            final initializing =
                                (scanProgress?.initialize ?? false) ||
                                    (analyseProgress?.initialize ?? false);

                            String fileName =
                                File(itemPath).uri.pathSegments.last;

                            void Function()? whileWorking(void Function() x) {
                              return isCurrentLibrary ||
                                      (initializing &&
                                          (scanWorking || analyseWorking))
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
                                          await closeLibrary(context);
                                          libraryPath.setLibraryPath(
                                              allOpenedFiles[index]);

                                          if (!context.mounted) return;
                                          context.go('/library');
                                        },
                                      ),
                                      child: const Text("Switch to"),
                                    ),
                                    const SizedBox(width: 12),
                                    Button(
                                      onPressed: whileWorking(() async {
                                        libraryPath.removeOpenedFile(
                                            allOpenedFiles[index]);
                                      }),
                                      child: const Text("Remove"),
                                    ),
                                    if (isCurrentLibrary) ...[
                                      const SizedBox(width: 12),
                                      const ScanLibraryButton(),
                                      const SizedBox(width: 12),
                                      const AnalyseLibraryButton(),
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
                      );
                    },
                  )
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
