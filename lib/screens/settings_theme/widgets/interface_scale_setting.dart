import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../providers/app_scale.dart';
import '../../../utils/dialogs/information/error.dart';
import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';

class InterfaceScaleSetting extends StatelessWidget {
  const InterfaceScaleSetting({super.key});

  Future<void> _setScale(BuildContext context, int value) async {
    try {
      await context.read<AppScaleProvider>().setPercentage(value);
    } catch (error) {
      if (!context.mounted) return;
      await showErrorDialog(
        context: context,
        title: S.of(context).interfaceScaleSaveError,
        errorMessage: error.toString(),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    final percentage = context.watch<AppScaleProvider>().percentage;
    return SettingsBoxComboBox<int>(
      title: s.interfaceScale,
      subtitle: s.interfaceScaleSubtitle,
      value: percentage,
      items: [
        for (final value in AppScaleProvider.supportedPercentages)
          SettingsBoxComboBoxItem(value: value, title: '$value%'),
      ],
      onChanged: (value) {
        if (value != null) _setScale(context, value);
      },
    );
  }
}
