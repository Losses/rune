import 'dart:io';

import 'package:player/widgets/library_task_button.dart';
import 'package:player/widgets/playback_controller/playback_placeholder.dart';
import 'package:provider/provider.dart';
import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:file_selector/file_selector.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';
import '../../messages/library_manage.pb.dart';
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

    List<String> allOpenedFiles = libraryPath.getAllOpenedFiles();

    return Column(children: [
      const NavigationBarPlaceholder(),
      Expanded(
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 24),
          child: SingleChildScrollView(
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8),
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
                      }),
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
                        final analyseProgress =
                            libraryManager.getAnalyseTaskProgress(itemPath);

                        final scanWorking =
                            scanProgress?.status == TaskStatus.working;
                        final analyseWorking =
                            analyseProgress?.status == TaskStatus.working;

                        final initializing =
                            (scanProgress?.initialize ?? false) ||
                                (analyseProgress?.initialize ?? false);

                        String fileName = File(itemPath).uri.pathSegments.last;

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
                                  onPressed: whileWorking(() async {
                                    await closeLibrary(context);
                                    libraryPath
                                        .setLibraryPath(allOpenedFiles[index]);

                                    if (!context.mounted) return;
                                    context.go('/library');
                                  }),
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
                  )
                ],
              ),
            ),
          ),
        ),
      ),
      const PlaybackPlaceholder(),
    ]);
  }
}

Future<void> closeLibrary(BuildContext context) async {
  final library = Provider.of<LibraryPathProvider>(context, listen: false);

  final path = library.currentPath;
  CloseLibraryRequest(path: path).sendSignalToRust();

  while (true) {
    final rustSignal = await CloseLibraryResponse.rustSignalStream.first;

    if (rustSignal.message.path == path) {
      return;
    }
  }
}
