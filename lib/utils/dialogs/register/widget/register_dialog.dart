import 'package:flutter_svg/svg.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:file_selector/file_selector.dart';

import '../../../../providers/license.dart';
import '../../../../widgets/no_shortcuts.dart';
import '../../../../widgets/responsive_dialog_actions.dart';

import '../../../api/register_license.dart';
import '../../../l10n.dart';

import '../../../settings_manager.dart';
import '../show_register_dialog.dart';

import 'show_register_failed_dialog.dart';
import 'show_register_invalid_dialog.dart';
import 'show_register_success_dialog.dart';

class RegisterDialog extends StatefulWidget {
  final void Function(void) $close;

  const RegisterDialog({super.key, required this.$close});

  @override
  State<RegisterDialog> createState() => _RegisterDialogState();
}

class _RegisterDialogState extends State<RegisterDialog> {
  bool _loading = false;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return NoShortcuts(
      ContentDialog(
        title: Column(
          children: [
            const SizedBox(height: 8),
            Text(S.of(context).evaluationMode),
          ],
        ),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              S.of(context).evaluationModeContent1,
              style: TextStyle(height: 1.4),
            ),
            SizedBox(height: 4),
            Text(
              S.of(context).evaluationModeContent2,
              style: TextStyle(height: 1.4),
            ),
            SizedBox(height: 4),
            Text(
              S.of(context).evaluationModeContent4,
              style: TextStyle(height: 1.4),
            ),
            SizedBox(height: 16),
            Button(
              onPressed: _loading ? null : () => {},
              child: Row(
                children: [
                  SvgPicture.asset(
                    width: 16,
                    'assets/bandcamp.svg',
                    colorFilter: ColorFilter.mode(
                      theme.inactiveColor,
                      BlendMode.srcIn,
                    ),
                  ),
                  SizedBox(width: 8),
                  Text("Bandcamp"),
                ],
              ),
            ),
            SizedBox(height: 4),
            Button(
              onPressed: _loading ? null : () => {},
              child: Row(
                children: [
                  SvgPicture.asset(
                    width: 16,
                    'assets/itch.svg',
                    colorFilter: ColorFilter.mode(
                      theme.inactiveColor,
                      BlendMode.srcIn,
                    ),
                  ),
                  SizedBox(width: 8),
                  Text("itch.io"),
                ],
              ),
            ),
          ],
        ),
        actions: [
          ResponsiveDialogActions(
            FilledButton(
              onPressed: _loading
                  ? null
                  : () async {
                      const XTypeGroup typeGroup = XTypeGroup(
                        label: 'Rune license',
                        extensions: <String>['flac'],
                      );
                      final XFile? file = await openFile(
                        acceptedTypeGroups: <XTypeGroup>[typeGroup],
                      );

                      if (file == null) return;

                      setState(() {
                        _loading = true;
                      });

                      final license = await registerLicense(file.path);

                      widget.$close(null);

                      if (!context.mounted) return;

                      if (license.success && license.valid) {
                        SettingsManager().setValue(licenseKey, license.license);
                        Provider.of<LicenseProvider>(context, listen: false)
                            .revalidateLicense();
                        showRegisterSuccessDialog(context);
                        return;
                      }

                      if (license.success && !license.valid) {
                        await showRegisterInvalidDialog(context);

                        if (!context.mounted) return;
                        showRegisterDialog(context);

                        return;
                      }
                      if (!license.success) {
                        showRegisterFailedDialog(context, license.error);

                        if (!context.mounted) return;
                        showRegisterDialog(context);

                        return;
                      }
                    },
              child: Text(S.of(context).register),
            ),
            Button(
              onPressed: _loading
                  ? null
                  : () {
                      widget.$close(null);
                    },
              child: Text(S.of(context).close),
            ),
          ),
        ],
      ),
    );
  }
}
