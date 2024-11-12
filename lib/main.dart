import 'dart:io';

import 'package:rinf/rinf.dart';
import 'package:provider/provider.dart';
import 'package:flutter/foundation.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:system_theme/system_theme.dart';
import 'package:flutter_acrylic/flutter_acrylic.dart';
import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter_fullscreen/flutter_fullscreen.dart';

import 'utils/platform.dart';
import 'utils/rune_log.dart';
import 'utils/settings_manager.dart';
import 'utils/update_color_mode.dart';
import 'utils/theme_color_manager.dart';
import 'utils/storage_key_manager.dart';
import 'utils/file_storage/mac_secure_manager.dart';

import 'config/theme.dart';
import 'config/routes.dart';
import 'config/app_title.dart';
import 'config/shortcuts.dart';

import 'widgets/router/no_effect_page_route.dart';
import 'widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

import 'screens/settings_theme/settings_theme.dart';

import 'messages/generated.dart';

import 'providers/crash.dart';
import 'providers/volume.dart';
import 'providers/status.dart';
import 'providers/playlist.dart';
import 'providers/full_screen.dart';
import 'providers/router_path.dart';
import 'providers/library_path.dart';
import 'providers/library_manager.dart';
import 'providers/playback_controller.dart';
import 'providers/responsive_providers.dart';

import 'theme.dart';
import 'widgets/shortcuts/router_actions_manager.dart';

late bool disableBrandingAnimation;
late String? initialPath;

void main(List<String> arguments) async {
  WidgetsFlutterBinding.ensureInitialized();

  String? profile = arguments.contains('--profile')
      ? arguments[arguments.indexOf('--profile') + 1]
      : null;

  await MacSecureManager().completed;
  StorageKeyManager.initialize(profile);

  await FullScreen.ensureInitialized();

  await initializeRust(assignRustSignal);

  try {
    final DeviceInfoPlugin deviceInfo = DeviceInfoPlugin();
    final windowsInfo = await deviceInfo.windowsInfo;
    final isWindows10 = windowsInfo.productName.startsWith('Windows 10');

    if (isWindows10 && appTheme.windowEffect == WindowEffect.mica) {
      appTheme.windowEffect = WindowEffect.solid;
    }
  } catch (e) {
    info$('Device is not Windows 10, skip the patch');
  }

  final SettingsManager settingsManager = SettingsManager();

  final String? colorMode =
      await settingsManager.getValue<String>(colorModeKey);

  updateColorMode(colorMode);

  await ThemeColorManager().initialize();

  final int? themeColor = await settingsManager.getValue<int>(themeColorKey);

  if (themeColor != null) {
    appTheme.updateThemeColor(Color(themeColor));
  }

  disableBrandingAnimation =
      await settingsManager.getValue<bool>(disableBrandingAnimationKey) ??
          false;

  bool? storedFullScreen =
      await settingsManager.getValue<bool>('fullscreen_state');
  if (storedFullScreen != null) {
    FullScreen.setFullScreen(storedFullScreen);
  }

  WidgetsFlutterBinding.ensureInitialized();

  // if it's not on the web, windows or android, load the accent color
  if (!kIsWeb &&
      [
        TargetPlatform.windows,
        TargetPlatform.android,
      ].contains(
        defaultTargetPlatform,
      )) {
    SystemTheme.accentColor.load();
  }

  if (isDesktop && !Platform.isLinux) {
    await Window.initialize();
  }

  if (!Platform.isLinux && !Platform.isAndroid) {
    appTheme.addListener(() {
      WidgetsBinding.instance.addPostFrameCallback(
        (_) {
          appTheme.setEffect();
        },
      );
    });
  }

  initialPath = await getInitialPath();

  runApp(
    MultiProvider(
      providers: [
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => CrashProvider(),
        ),
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => VolumeProvider(),
        ),
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => PlaylistProvider(),
        ),
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => ScreenSizeProvider(),
        ),
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => PlaybackStatusProvider(),
        ),
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => LibraryPathProvider(initialPath),
        ),
        ChangeNotifierProxyProvider<ScreenSizeProvider, ResponsiveProvider>(
          create: (context) =>
              ResponsiveProvider(context.read<ScreenSizeProvider>()),
          update: (context, screenSizeProvider, previous) =>
              previous ?? ResponsiveProvider(screenSizeProvider),
        ),
        ChangeNotifierProvider(create: (_) => $router),
        ChangeNotifierProvider(create: (_) => PlaybackControllerProvider()),
        ChangeNotifierProvider(create: (_) => LibraryManagerProvider()),
        ChangeNotifierProvider(create: (_) => FullScreenProvider()),
      ],
      child: const Rune(),
    ),
  );
}

final rootNavigatorKey = GlobalKey<NavigatorState>();

class Rune extends StatelessWidget {
  const Rune({super.key});

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider.value(
      value: appTheme,
      builder: (context, child) {
        final appTheme = context.watch<AppTheme>();

        return FluentApp(
          title: appTitle,
          initialRoute: initialPath == null ? "/" : "/library",
          navigatorKey: rootNavigatorKey,
          onGenerateRoute: (settings) {
            final routeName = settings.name!;

            if (routeName == '/') {
              return NoEffectPageRoute<dynamic>(
                settings: settings,
                builder: (context) => routes["/"]!(context),
              );
            }

            if (routeName == '/scanning') {
              return NoEffectPageRoute<dynamic>(
                settings: settings,
                builder: (context) => routes["/scanning"]!(context),
              );
            }

            final page = RuneWithNavigationBarAndPlaybackControllor(
              routeName: routeName,
            );

            return NoEffectPageRoute<dynamic>(
              settings: settings,
              builder: (context) => page,
            );
          },
          debugShowCheckedModeBanner: false,
          color: appTheme.color,
          themeMode: appTheme.mode,
          theme: FluentThemeData(
            accentColor: appTheme.color,
            visualDensity: VisualDensity.standard,
          ),
          darkTheme: FluentThemeData(
            brightness: Brightness.dark,
            accentColor: appTheme.color,
            visualDensity: VisualDensity.standard,
          ),
          locale: appTheme.locale,
          builder: (context, child) {
            final theme = FluentTheme.of(context);

            return Container(
              color: appTheme.windowEffect == WindowEffect.solid
                  ? theme.micaBackgroundColor
                  : Colors.transparent,
              child: Directionality(
                textDirection: appTheme.textDirection,
                child: Shortcuts(
                  shortcuts: shortcuts,
                  child: NavigationShortcutManager(child!),
                ),
              ),
            );
          },
        );
      },
    );
  }
}
