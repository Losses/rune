import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/api/server_availability_test_request.dart';
import '../../utils/dialogs/failed_to_initialize_library.dart';
import '../../utils/dialogs/information/error.dart';
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
    final s = S.of(context);
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
                        final item = allOpenedFiles[index];
                        final isCurrentLibrary =
                            item.rawPath == libraryPath.currentPath;
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
                                          try {
                                            await serverAvailabilityTest(
                                              item.cleanPath,
                                            );
                                          } catch (e) {
                                            if (!context.mounted) return;
                                            showErrorDialog(
                                              context: context,
                                              title: s.unknownError,
                                              errorMessage: e.toString(),
                                            );
                                            return;
                                          }

                                          if (!context.mounted) return;
                                          await closeLibrary(context);

                                          if (!context.mounted) return;

                                          final (success, notReady, error) =
                                              await libraryPath.setLibraryPath(
                                            context,
                                            allOpenedFiles[index].rawPath,
                                            null,
                                          );

                                          if (!success) {
                                            showFailedToInitializeLibrary(
                                              context,
                                              error,
                                            );

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
