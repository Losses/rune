import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:file_selector/file_selector.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/api/close_library.dart';
import '../../../utils/dialogs/select_library_mode/test_and_select_library_mode.dart';
import '../../../utils/router/navigation.dart';
import '../../../utils/dialogs/failed_to_initialize_library.dart';
import '../../../providers/library_manager.dart';
import '../../../providers/library_path.dart';
import '../../../utils/l10n.dart';

import 'settings_button.dart';

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
        final path = await getDirectoryPath();

        if (path == null) return;
        if (!context.mounted) return;

        final result = await testAndSelectLibraryMode(context, path);

        if (result == null) return;
        final (initialized, initializeMode) = result;
        if (!initialized && initializeMode == null) return;

        if (tryClose) {
          if (!context.mounted) return;
          await closeLibrary(context);
        }

        if (!context.mounted) return;

        final (switched, cancelled, error) =
            await libraryPath.setLibraryPath(context, path, initializeMode);

        if (switched) {
          libraryManager.scanLibrary(path, true);
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
