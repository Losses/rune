import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/get_dir_path.dart';
import '../../../utils/api/close_library.dart';
import '../../../utils/router/navigation.dart';
import '../../../utils/dialogs/failed_to_initialize_library.dart';
import '../../../widgets/settings/settings_button.dart';
import '../../../providers/library_manager.dart';
import '../../../providers/library_path.dart';

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
    final libraryPath = Provider.of<LibraryPathProvider>(context, listen: true);
    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: false);

    return SettingsButton(
      icon: Symbols.add,
      title: S.of(context).addLibrary,
      subtitle: S.of(context).addLibrarySubtitle,
      onPressed: () async {
        final path = await getDirPath();

        if (path == null) return;
        if (!context.mounted) return;

        if (tryClose) {
          if (!context.mounted) return;
          await closeLibrary(context);
        }

        if (!context.mounted) return;

        final (switched, cancelled, error) =
            await libraryPath.setLibraryPath(context, path, null);

        if (switched) {
          libraryManager.scanLibrary(
            path,
            isInitializeTask: true,
          );
        } else if (!cancelled) {
          if (!context.mounted) return;
          await showFailedToInitializeLibrary(context, error);
          if (navigateIfFailed) {
            $$replace('/');
          }
        }
      },
    );
  }
}
