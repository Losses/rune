import 'dart:io';

import 'package:file_selector/file_selector.dart';
import 'package:player/providers/library_manager.dart';
import 'package:provider/provider.dart';
import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/navigation_bar.dart';
import '../../messages/library_manage.pb.dart';
import '../../providers/library_path.dart';

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
    final provider = Provider.of<LibraryPathProvider>(context, listen: true);
    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: true);

    List<String> allOpenedFiles = provider.getAllOpenedFiles();

    return ScaffoldPage(
      content: Column(children: [
        const NavigationBarPlaceholder(),
        Center(
          child: Column(
            children: [
              SettingsButton(
                  icon: Symbols.add,
                  title: "Add Library",
                  subtitle: "Add a new library and scan existing files",
                  onPressed: () async {
                    final result = await getDirectoryPath();

                    if (result == null) return;

                    libraryManager.scanLibrary(result, true);
                  }),
              SettingsButton(
                  icon: Symbols.refresh,
                  title: "Factory Reset",
                  subtitle: "Remove all items from the library list",
                  onPressed: () async {
                    await closeLibrary(context);

                    provider.clearAllOpenedFiles();
                  }),
              const SizedBox(height: 2),
              SizedBox(
                width: double.maxFinite,
                child: ListView.builder(
                  shrinkWrap: true,
                  itemCount: allOpenedFiles.length,
                  itemBuilder: (context, index) {
                    final itemPath = allOpenedFiles[index];
                    final isCurrentLibrary = itemPath == provider.currentPath;
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
                                      provider.setLibraryPath(
                                          allOpenedFiles[index]);

                                      if (!context.mounted) return;
                                      context.go('/library');
                                    },
                              child: const Text("Switch to"),
                            ),
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
      ]),
    );
  }
}

class SettingsButton extends StatelessWidget {
  const SettingsButton({
    super.key,
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.onPressed,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final void Function()? onPressed;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(4),
      child: Button(
        style: ButtonStyle(
            shape: WidgetStateProperty.all(RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(4)))),
        onPressed: onPressed,
        child: SettingsTileTitle(
          icon: icon,
          title: title,
          subtitle: subtitle,
          showActions: false,
          actionsBuilder: (context) => Container(),
        ),
      ),
    );
  }
}

class ProgressButton extends StatelessWidget {
  final Widget Function()? onPressed;

  const ProgressButton({
    super.key,
    required this.title,
    required this.onPressed,
  });

  final String title;

  @override
  Widget build(BuildContext context) {
    return Button(
        onPressed: onPressed,
        child: Row(
          children: [
            const SizedBox(
              width: 16,
              height: 16,
              child: OverflowBox(
                  maxWidth: 16,
                  maxHeight: 16,
                  child: ProgressRing(
                    strokeWidth: 2,
                  )),
            ),
            const SizedBox(
              width: 8,
            ),
            Text(title)
          ],
        ));
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
