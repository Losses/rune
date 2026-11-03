import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../utils/remote_connection_messages.dart';
import '../../utils/api/close_library.dart';
import '../../utils/router/navigation.dart';
import '../../utils/dialogs/failed_to_initialize_library.dart';
import '../../providers/library_path.dart';

import '../settings_neighbors/widgets/settings_tile_title.dart';

class RemoteDestinationsList extends StatefulWidget {
  const RemoteDestinationsList({super.key});

  @override
  State<RemoteDestinationsList> createState() => _RemoteDestinationsListState();
}

class _RemoteDestinationsListState extends State<RemoteDestinationsList> {
  String selectedItem = '';

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    final libraryPath = Provider.of<LibraryPathProvider>(context, listen: true);
    final allOpenedFiles = libraryPath.getAnyDestinationRemotePaths();

    return SizedBox(
      width: double.maxFinite,
      child: ListView.builder(
        shrinkWrap: true,
        physics: const NeverScrollableScrollPhysics(),
        itemCount: allOpenedFiles.length,
        itemBuilder: (context, index) {
          final item = allOpenedFiles[index];
          final isCurrentLibrary = item.rawPath == libraryPath.currentPath;
          final isSelectedLibrary = item.rawPath == selectedItem;

          return ListTile.selectable(
            title: SettingsTileTitle(
              icon: Symbols.devices,
              title: item.alias,
              subtitle: item.cleanPath,
              showActions: isSelectedLibrary,
              actionsBuilder: (context) => Row(
                children: [
                  Button(
                    onPressed: isCurrentLibrary
                        ? null
                        : () async {
                            await closeLibrary(context);

                            if (!context.mounted) return;

                            final (success, _, _, errorKind) = await libraryPath
                                .setLibraryPath(
                                  context,
                                  allOpenedFiles[index].rawPath,
                                  null,
                                );

                            if (!success) {
                              await showFailedToInitializeLibrary(
                                context,
                                remoteOpenErrorMessage(s, errorKind),
                              );

                              $$replace("/");

                              return;
                            }

                            if (!context.mounted) return;
                            $push('/library');
                          },
                    child: Text(s.switchTo),
                  ),
                  const SizedBox(width: 12),
                  Button(
                    onPressed: isCurrentLibrary
                        ? null
                        : () async {
                            libraryPath.removeOpenedFile(
                              allOpenedFiles[index].rawPath,
                            );
                          },
                    child: Text(s.removeLibrary),
                  ),
                ],
              ),
            ),
            selected: isSelectedLibrary,
            onSelectionChange: (v) =>
                setState(() => selectedItem = item.rawPath),
          );
        },
      ),
    );
  }
}
