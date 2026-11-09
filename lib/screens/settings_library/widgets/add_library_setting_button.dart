import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/get_dir_path.dart';
import '../../../utils/router/navigation.dart';
import '../../../utils/dialogs/failed_to_initialize_library.dart';
import '../../../widgets/settings/settings_button.dart';
import '../../../providers/library_manager.dart';
import '../../../providers/library_registry.dart';
import '../../../utils/library_registry_messages.dart';

class AddLibrarySettingButton extends StatelessWidget {
  const AddLibrarySettingButton({
    super.key,
    required this.tryClose,
    required this.navigateIfFailed,
  });

  final bool tryClose;
  final bool navigateIfFailed;

  @override
  Widget build(BuildContext context) {
    final registry = Provider.of<LibraryRegistryProvider>(
      context,
      listen: false,
    );
    final libraryManager = Provider.of<LibraryManagerProvider>(
      context,
      listen: false,
    );

    return SettingsButton(
      icon: Symbols.add,
      title: S.of(context).addLibrary,
      subtitle: S.of(context).addLibrarySubtitle,
      onPressed: () async {
        final path = await getDirPath();

        if (path == null) return;
        if (!context.mounted) return;

        final alias = path.split('/').where((part) => part.isNotEmpty).last;
        try {
          final record = await registerAndOpenLocalFolder(
            context,
            registry,
            path,
            alias,
          );
          if (record == null) return;
          libraryManager.scanLibrary(record.recordId, isInitializeTask: true);
        } catch (error) {
          if (!context.mounted) return;
          await showFailedToInitializeLibrary(
            context,
            error is LibraryRegistryFailure
                ? libraryRegistryFailureMessage(S.of(context), error.kind)
                : S.of(context).unknownError,
          );
          if (navigateIfFailed) {
            $$replace('/');
          }
        }
      },
    );
  }
}
