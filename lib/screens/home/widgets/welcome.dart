import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:file_selector/file_selector.dart';
import 'package:permission_handler/permission_handler.dart';

import '../../../utils/ax_shadow.dart';
import '../../../utils/dialogs/failed_to_initialize_library.dart';
import '../../../providers/library_path.dart';
import '../../../providers/library_manager.dart';
import '../../../providers/responsive_providers.dart';
import '../../../utils/l10n.dart';

class WelcomePage extends StatelessWidget {
  const WelcomePage({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    final r = Provider.of<ResponsiveProvider>(context);

    final libraryPath =
        Provider.of<LibraryPathProvider>(context, listen: false);
    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: false);

    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 400, maxHeight: 560),
        child: Container(
          decoration: BoxDecoration(
            color: theme.cardColor,
            borderRadius: BorderRadius.circular(3),
            boxShadow: axShadow(20),
          ),
          child: Padding(
            padding: const EdgeInsets.all(12),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.start,
              children: [
                Expanded(
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    crossAxisAlignment: CrossAxisAlignment.center,
                    children: [
                      SvgPicture.asset(
                        'assets/mono_color_logo.svg',
                        colorFilter: ColorFilter.mode(
                          theme.inactiveColor,
                          BlendMode.srcIn,
                        ),
                      ),
                      r.smallerOrEqualTo(DeviceType.car, false)
                          ? r.smallerOrEqualTo(DeviceType.band, false)
                              ? const SizedBox(height: 2)
                              : const SizedBox(height: 14)
                          : const SizedBox(height: 20),
                      Column(
                        children: [
                          if (!r.smallerOrEqualTo(DeviceType.band, false))
                            Padding(
                              padding:
                                  const EdgeInsets.symmetric(horizontal: 24),
                              child: Text(
                                S.of(context).selectLibraryDirectorySubtitle,
                                textAlign: TextAlign.center,
                                style: TextStyle(height: 1.4),
                              ),
                            ),
                          r.smallerOrEqualTo(DeviceType.car, false)
                              ? const SizedBox(height: 20)
                              : const SizedBox(height: 56),
                          FilledButton(
                            child: Text(S.of(context).selectDirectory),
                            onPressed: () async {
                              // on Android, check for permission
                              if (!await requestAndroidPermission()) return;

                              final path = await getDirectoryPath();

                              if (path == null) return;
                              if (!context.mounted) return;

                              final (success, error) =
                                  await libraryPath.setLibraryPath(path);

                              if (success) {
                                libraryManager.scanLibrary(path, true);
                              } else {
                                if (!context.mounted) return;
                                await showFailedToInitializeLibrary(
                                  context,
                                  error,
                                );
                              }
                            },
                          ),
                        ],
                      ),
                    ],
                  ),
                ),
                if (!r.smallerOrEqualTo(DeviceType.belt, false))
                  Text(
                    S.of(context).copyrightAnnouncement,
                    style: theme.typography.caption
                        ?.apply(color: theme.inactiveColor.withAlpha(80)),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  static Future<bool> requestAndroidPermission() async {
    if (Platform.isAndroid) {
      PermissionStatus status = await Permission.manageExternalStorage.status;
      if (status.isDenied) {
        return await Permission.manageExternalStorage.request().isGranted;
      }
    }
    return true;
  }
}
