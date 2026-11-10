import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/dialogs/add_library/add_library_guide.dart';
import '../../../widgets/settings/settings_button.dart';

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
    return SettingsButton(
      icon: Symbols.add,
      title: S.of(context).addLibrary,
      subtitle: S.of(context).addLibrarySubtitle,
      onPressed: () =>
          showAddLibraryGuide(context, navigateIfFailed: navigateIfFailed),
    );
  }
}
