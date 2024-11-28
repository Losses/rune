import 'dart:io';

import 'package:flutter_svg/svg.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/dialogs/failed_to_initialize_library.dart';
import '../../../providers/library_path.dart';
import '../../../providers/responsive_providers.dart';

import '../../settings_library/widgets/add_library_setting_button.dart';
import '../../settings_library/widgets/settings_button.dart';

class SelectLibraryPage extends StatefulWidget {
  const SelectLibraryPage({super.key});

  @override
  State<SelectLibraryPage> createState() => _SelectLibraryPageState();
}

class _SelectLibraryPageState extends State<SelectLibraryPage> {
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
        allOpenedFiles = x;
      });
    });

    super.didChangeDependencies();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    final r = Provider.of<ResponsiveProvider>(context);

    final libraryPath =
        Provider.of<LibraryPathProvider>(context, listen: false);

    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 400, maxHeight: 560),
        child: Container(
          padding: const EdgeInsets.all(8),
          child: Column(
            children: [
              if (!r.smallerOrEqualTo(DeviceType.belt, false))
                SvgPicture.asset(
                  'assets/mono_color_logo.svg',
                  colorFilter: ColorFilter.mode(
                    theme.inactiveColor,
                    BlendMode.srcIn,
                  ),
                ),
              DeviceTypeBuilder(
                deviceType: const [
                  DeviceType.band,
                  DeviceType.belt,
                  DeviceType.car,
                  DeviceType.station
                ],
                builder: (context, activeBreakpoint) {
                  switch (activeBreakpoint) {
                    case DeviceType.station:
                      return const SizedBox(height: 20);
                    case DeviceType.car:
                      return const SizedBox(height: 14);
                    case DeviceType.belt:
                      return const SizedBox(height: 2);
                    case DeviceType.band:
                      return const SizedBox(height: 0);
                    default:
                      return const SizedBox(height: 20);
                  }
                },
              ),
              Expanded(
                child: SingleChildScrollView(
                  child: SizedBox(
                    width: double.maxFinite,
                    child: ListView.builder(
                      shrinkWrap: true,
                      itemCount: allOpenedFiles.length + 1,
                      itemBuilder: (context, index) {
                        if (index == 0) {
                          return const AddLibrarySettingButton(
                            tryClose: false,
                            navigateIfFailed: false,
                          );
                        }

                        final itemPath = allOpenedFiles[index - 1];

                        String fileName = File(itemPath).uri.pathSegments.last;

                        return SettingsButton(
                          icon: Symbols.folder,
                          title: fileName,
                          subtitle: allOpenedFiles[index - 1],
                          onPressed: () async {
                            final path = allOpenedFiles[index - 1];

                            final (switched, cancelled, error) =
                                await libraryPath.setLibraryPath(
                              context,
                              path,
                              null,
                            );

                            if (!context.mounted) return;
                            if (switched) return;
                            if (cancelled) return;

                            await showFailedToInitializeLibrary(
                              context,
                              error,
                            );
                          },
                        );
                      },
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
