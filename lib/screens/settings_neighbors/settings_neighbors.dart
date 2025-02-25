import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../utils/settings_page_padding.dart';
import '../../utils/settings_body_padding.dart';
import '../../utils/api/close_library.dart';
import '../../utils/router/navigation.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../providers/library_path.dart';

import 'widgets/add_neighbor_manually_setting_button.dart';
import 'widgets/edit_device_information_setting_button.dart';
import 'widgets/search_remote_device_setting_button.dart';
import 'widgets/settings_tile_title.dart';

class SettingsNeighborsPage extends StatefulWidget {
  const SettingsNeighborsPage({super.key});

  @override
  State<SettingsNeighborsPage> createState() => _SettingsNeighborsPageState();
}

class _SettingsNeighborsPageState extends State<SettingsNeighborsPage> {
  String selectedItem = '';

  bool requested = false;
  List<LibraryPathEntry> allOpenedFiles = [];

  @override
  void didChangeDependencies() {
    if (requested) return;

    final libraryPath =
        Provider.of<LibraryPathProvider>(context, listen: false);

    setState(() {
      allOpenedFiles = libraryPath.getAnyDestinationRemotePaths();
    });

    super.didChangeDependencies();
  }

  @override
  Widget build(BuildContext context) {
    final libraryPath = Provider.of<LibraryPathProvider>(context, listen: true);

    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SettingsPagePadding(
          child: SingleChildScrollView(
            padding: getScrollContainerPadding(context),
            child: SettingsBodyPadding(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const SearchRemoteDeviceSettingButton(
                    tryClose: true,
                    navigateIfFailed: true,
                  ),
                  const AddNeighborManuallySettingButton(
                    tryClose: true,
                    navigateIfFailed: true,
                  ),
                  const EditDeviceInformationSettingButton(),
                  const SizedBox(height: 2),
                  SizedBox(
                    width: double.maxFinite,
                    child: ListView.builder(
                      shrinkWrap: true,
                      itemCount: allOpenedFiles.length,
                      itemBuilder: (context, index) {
                        final itemPath = allOpenedFiles[index].rawPath;
                        final isCurrentLibrary =
                            itemPath == libraryPath.currentPath;
                        final isSelectedLibrary = itemPath == selectedItem;

                        final String fileName =
                            File(itemPath).uri.pathSegments.last;

                        return ListTile.selectable(
                          title: SettingsTileTitle(
                            icon: Symbols.devices,
                            title: fileName,
                            subtitle: allOpenedFiles[index].cleanPath,
                            showActions: isSelectedLibrary,
                            actionsBuilder: (context) => Row(
                              children: [
                                Button(
                                  onPressed: isCurrentLibrary
                                      ? null
                                      : () async {
                                          if (!context.mounted) return;
                                          await closeLibrary(context);

                                          if (!context.mounted) return;

                                          libraryPath.setLibraryPath(
                                            context,
                                            allOpenedFiles[index].rawPath,
                                            null,
                                          );

                                          if (!context.mounted) return;
                                          $push('/library');
                                        },
                                  child: Text(S.of(context).switchTo),
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
                                  child: Text(S.of(context).removeLibrary),
                                ),
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
