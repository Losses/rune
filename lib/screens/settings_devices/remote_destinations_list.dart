import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../utils/router/navigation.dart';
import '../../utils/dialogs/failed_to_initialize_library.dart';
import '../../utils/dialogs/subsonic/subsonic_relogin_dialog.dart';
import '../../providers/library_registry.dart';
import '../../bindings/bindings.dart';
import '../../utils/library_registry_messages.dart';

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
    final registry = Provider.of<LibraryRegistryProvider>(context);
    final allRecords = registry.records
        .where(
          (record) => switch (record.kind) {
            LibraryRecordKindSummaryReplica() ||
            LibraryRecordKindSummaryRemoteControl() => true,
            _ => false,
          },
        )
        .toList();

    return SizedBox(
      width: double.maxFinite,
      child: ListView.builder(
        shrinkWrap: true,
        physics: const NeverScrollableScrollPhysics(),
        itemCount: allRecords.length,
        itemBuilder: (context, index) {
          final item = allRecords[index];
          final path = switch (item.kind) {
            LibraryRecordKindSummaryReplica() => item.alias,
            LibraryRecordKindSummaryRemoteControl(:final endpoint) =>
              endpoint ?? '',
            _ => '',
          };
          final isCurrentLibrary = item.recordId == registry.openRecordId;
          final isSelectedLibrary = item.recordId == selectedItem;

          return ListTile.selectable(
            title: SettingsTileTitle(
              icon: Symbols.devices,
              title: item.alias,
              subtitle: path,
              showActions: isSelectedLibrary,
              actionsBuilder: (context) => Row(
                children: [
                  Button(
                    onPressed: () async {
                      try {
                        await openLibraryWithSubsonicRelogin(
                          context,
                          registry,
                          item.recordId,
                        );
                      } catch (error) {
                        if (!context.mounted) return;
                        await showFailedToInitializeLibrary(
                          context,
                          error is LibraryRegistryFailure
                              ? libraryRegistryFailureMessage(s, error.kind)
                              : s.unknownError,
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
                            try {
                              await registry.remove(item.recordId);
                            } catch (error) {
                              if (!context.mounted) return;
                              await showFailedToInitializeLibrary(
                                context,
                                error is LibraryRegistryFailure
                                    ? libraryRegistryFailureMessage(
                                        s,
                                        error.kind,
                                      )
                                    : s.unknownError,
                              );
                            }
                          },
                    child: Text(s.removeLibrary),
                  ),
                ],
              ),
            ),
            selected: isSelectedLibrary,
            onSelectionChange: (v) =>
                setState(() => selectedItem = item.recordId),
          );
        },
      ),
    );
  }
}
