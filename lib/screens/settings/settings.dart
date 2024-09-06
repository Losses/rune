import 'dart:io';

import 'package:file_selector/file_selector.dart';
import 'package:provider/provider.dart';
import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/navigation_bar.dart';
import '../../messages/library_manage.pb.dart';
import '../../providers/library_path.dart';
import '../../providers/library_manager.dart';

import './widgets/settings_button.dart';
import './widgets/progress_button.dart';
import './widgets/settings_tile_title.dart';

class SettingsPage extends StatefulWidget {
  const SettingsPage({super.key});

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  String selectedItem = '';

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    final libraryPath = Provider.of<LibraryPathProvider>(context, listen: true);
    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: true);

    List<String> allOpenedFiles = libraryPath.getAllOpenedFiles();

    return Column(children: [
      const NavigationBarPlaceholder(),
      Padding(
        padding: const EdgeInsets.symmetric(vertical: 24, horizontal: 32),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text("Library", style: typography.title),
            const SizedBox(height: 24),
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
                  final isCurrentLibrary = itemPath == libraryPath.currentPath;
                  final isSelectedLibrary = itemPath == selectedItem;

                  final scanProgress =
                      libraryManager.getScanTaskProgress(itemPath);
                  final analyseProgress =
                      libraryManager.getAnalyseTaskProgress(itemPath);

                  final scanWorking =
                      scanProgress?.status == TaskStatus.working;
                  final analyseWorking =
                      analyseProgress?.status == TaskStatus.working;

                  final initializing = (scanProgress?.initialize ?? false) ||
                      (analyseProgress?.initialize ?? false);

                  String fileName = File(itemPath).uri.pathSegments.last;

                  return ListTile.selectable(
                    title: SettingsTileTitle(
                      icon: Symbols.folder,
                      title: fileName,
                      subtitle: allOpenedFiles[index],
                      showActions: isSelectedLibrary,
                      actionsBuilder: (context) => Row(
                        children: [
                          Button(
                            onPressed: isCurrentLibrary ||
                                    (initializing &&
                                        (scanWorking || analyseWorking))
                                ? null
                                : () async {
                                    await closeLibrary(context);
                                    libraryPath
                                        .setLibraryPath(allOpenedFiles[index]);

                                    if (!context.mounted) return;
                                    context.go('/library');
                                  },
                            child: const Text("Switch to"),
                          ),
                          const SizedBox(
                            width: 12,
                          ),
                          Button(
                            onPressed: isCurrentLibrary ||
                                    (initializing &&
                                        (scanWorking || analyseWorking))
                                ? null
                                : () async {
                                    libraryPath.removeOpenedFile(
                                        allOpenedFiles[index]);
                                  },
                            child: const Text("Remove"),
                          ),
                          if (isCurrentLibrary) ...[
                            const SizedBox(
                              width: 12,
                            ),
                            scanWorking
                                ? const ProgressButton(
                                    title: "Scanning",
                                    onPressed: null,
                                  )
                                : Button(
                                    onPressed: analyseWorking
                                        ? null
                                        : () => libraryManager.scanLibrary(
                                            itemPath, false),
                                    child: const Text("Scan"),
                                  ),
                            const SizedBox(
                              width: 12,
                            ),
                            analyseWorking
                                ? const ProgressButton(
                                    title: "Analysing",
                                    onPressed: null,
                                  )
                                : Button(
                                    onPressed: scanWorking
                                        ? null
                                        : () => libraryManager.analyseLibrary(
                                            itemPath, false),
                                    child: const Text("Analyse"),
                                  )
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
      )
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
