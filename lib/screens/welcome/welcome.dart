import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:file_selector/file_selector.dart';
import 'package:permission_handler/permission_handler.dart';

import '../../utils/ax_shadow.dart';
import '../../utils/scan_library.dart';
import '../../providers/responsive_providers.dart';

class WelcomePage extends StatelessWidget {
  const WelcomePage({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    final r = Provider.of<ResponsiveProvider>(context);

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
                            const Padding(
                              padding: EdgeInsets.symmetric(horizontal: 24),
                              child: Text(
                                'Select your audio library directory, and we will scan and analysis all tracks within it.',
                                textAlign: TextAlign.center,
                                style: TextStyle(height: 1.4),
                              ),
                            ),
                          r.smallerOrEqualTo(DeviceType.car, false)
                              ? const SizedBox(height: 20)
                              : const SizedBox(height: 56),
                          FilledButton(
                            child: const Text("Select Directory"),
                            onPressed: () async {
                              // on Android, check for permission
                              if (!await requestAndroidPermission()) return;

                              final result = await getDirectoryPath();

                              if (result == null) return;

                              if (!context.mounted) return;

                              await scanLibrary(context, result);

                              if (!context.mounted) return;
                              context.go("/library");
                            },
                          ),
                        ],
                      ),
                    ],
                  ),
                ),
                if (!r.smallerOrEqualTo(DeviceType.belt, false))
                  Text(
                    '© 2024 Rune Player Developers. Licensed under MPL 2.0.',
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

  Future<bool> requestAndroidPermission() async {
    if (Platform.isAndroid) {
      PermissionStatus status = await Permission.manageExternalStorage.status;
      if (status.isDenied) {
        return await Permission.manageExternalStorage.request().isGranted;
      }
    }
    return true;
  }
}
