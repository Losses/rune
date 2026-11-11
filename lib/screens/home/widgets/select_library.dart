import 'package:flutter_svg/svg.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/router/navigation.dart';
import '../../../utils/library_ui/library_ui_state.dart';
import '../../../bindings/bindings.dart';
import '../../../utils/library_registry_messages.dart';
import '../../../utils/dialogs/failed_to_initialize_library.dart';
import '../../../utils/dialogs/subsonic/subsonic_relogin_dialog.dart';
import '../../../widgets/settings/settings_button.dart';
import '../../../providers/library_registry.dart';
import '../../../providers/responsive_providers.dart';

import '../../settings_library/widgets/add_library_setting_button.dart';

class SelectLibraryPage extends StatefulWidget {
  const SelectLibraryPage({super.key});

  @override
  State<SelectLibraryPage> createState() => _SelectLibraryPageState();
}

class _SelectLibraryPageState extends State<SelectLibraryPage> {
  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    final r = Provider.of<ResponsiveProvider>(context);

    final registry = Provider.of<LibraryRegistryProvider>(context);
    final records = registry.records
        .where(
          (record) => switch (record.kind) {
            LibraryRecordKindSummaryLocal() ||
            LibraryRecordKindSummaryReplica() => true,
            _ => false,
          },
        )
        .toList();

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
                  DeviceType.station,
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
                      // The list is in the middle of the window, not at its
                      // edges.
                      padding: EdgeInsets.zero,
                      shrinkWrap: true,
                      itemCount: records.length + 1,
                      itemBuilder: (context, index) {
                        if (index == 0) {
                          return const AddLibrarySettingButton(
                            tryClose: false,
                            navigateIfFailed: false,
                          );
                        }

                        final record = records[index - 1];
                        final itemPath = switch (record.kind) {
                          LibraryRecordKindSummaryLocal(:final path) => path,
                          _ => '',
                        };
                        final fileName = switch (record.kind) {
                          LibraryRecordKindSummaryReplica() => record.alias,
                          _ => localFolderName(itemPath),
                        };

                        return SettingsButton(
                          icon: Symbols.folder,
                          title: fileName,
                          subtitle: describeLibraryPath(
                            S.of(context),
                            itemPath,
                          ),
                          onPressed: () async {
                            try {
                              await openLibraryWithSubsonicRelogin(
                                context,
                                registry,
                                record.recordId,
                              );
                              $$replace('/library');
                            } catch (error) {
                              if (!context.mounted) return;
                              await showFailedToInitializeLibrary(
                                context,
                                error is LibraryRegistryFailure
                                    ? libraryRegistryFailureMessage(
                                        S.of(context),
                                        error.kind,
                                      )
                                    : S.of(context).unknownError,
                              );
                            }
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
