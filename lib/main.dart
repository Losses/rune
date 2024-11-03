import 'dart:io';

import 'package:rinf/rinf.dart';
import 'package:provider/provider.dart';
import 'package:flutter/foundation.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:get_storage/get_storage.dart';
import 'package:system_theme/system_theme.dart';
import 'package:flutter_acrylic/flutter_acrylic.dart';
import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter_fullscreen/flutter_fullscreen.dart';

import 'utils/platform.dart';
import 'utils/settings_manager.dart';
import 'utils/update_color_mode.dart';
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

late bool disableBrandingAnimation;

void main(List<String> arguments) async {
  WidgetsFlutterBinding.ensureInitialized();

  String? profile = arguments.contains('--profile')
      ? arguments[arguments.indexOf('--profile') + 1]
      : null;

  StorageKeyManager.initialize(profile);

  await FullScreen.ensureInitialized();
  await GetStorage.init();
  await GetStorage.init(MacSecureManager.storageName);
  await MacSecureManager.shared.loadBookmark();
  await initializeRust(assignRustSignal);

  try {
    final DeviceInfoPlugin deviceInfo = DeviceInfoPlugin();
    final windowsInfo = await deviceInfo.windowsInfo;
    final isWindows10 = windowsInfo.productName.startsWith('Windows 10');

    if (isWindows10 && appTheme.windowEffect == WindowEffect.mica) {
      appTheme.windowEffect = WindowEffect.solid;
    }
  } catch (e) {
    debugPrint('Device is not Windows 10, skip the patch');
  }

  final SettingsManager settingsManager = SettingsManager();

  String? colorMode = await settingsManager.getValue<String>(colorModeKey);

  updateColorMode(colorMode);

  int? themeColor = await settingsManager.getValue<int>(themeColorKey);

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

  runApp(
    MultiProvider(
      providers: [
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => LibraryPathProvider(),
        ),
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => VolumeProvider(),
        ),
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => ScreenSizeProvider(),
        ),
        ChangeNotifierProxyProvider<ScreenSizeProvider, ResponsiveProvider>(
          create: (context) =>
              ResponsiveProvider(context.read<ScreenSizeProvider>()),
          update: (context, screenSizeProvider, previous) =>
              previous ?? ResponsiveProvider(screenSizeProvider),
        ),
        ChangeNotifierProvider(create: (_) => $router),
        ChangeNotifierProvider(create: (_) => CrashProvider()),
        ChangeNotifierProvider(create: (_) => PlaylistProvider()),
        ChangeNotifierProvider(create: (_) => PlaybackControllerProvider()),
        ChangeNotifierProvider(create: (_) => PlaybackStatusProvider()),
        ChangeNotifierProvider(create: (_) => LibraryManagerProvider()),
        ChangeNotifierProvider(create: (_) => FullScreenProvider()),
      ],
      child: const Rune(),
    ),
  );
}

class Rune extends StatefulWidget {
  const Rune({super.key});

  @override
  State<Rune> createState() => _RuneState();
}

class _RuneState extends State<Rune> {
  void _updateWindowEffectCallback() {
    if (Platform.isLinux) return;
    if (Platform.isAndroid) return;
  }

  void updateWindowEffect() {
    if (Platform.isLinux) return;
    if (Platform.isAndroid) return;

    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        appTheme.setEffect(context);
      }
    });
  }

  @override
  void initState() {
    super.initState();
    appTheme.addListener(_updateWindowEffectCallback);
  }

  @override
  void dispose() {
    appTheme.removeListener(_updateWindowEffectCallback);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider.value(
      value: appTheme,
      builder: (context, child) {
        final appTheme = context.watch<AppTheme>();

        return FluentApp(
          title: appTitle,
          onGenerateRoute: (settings) {
            final routeName = settings.name!;

            if (routeName == '/') {
              final builder = routes["/"]!;
              final page = builder(context);

              return NoEffectPageRoute<dynamic>(
                settings: settings,
                builder: (context) => page,
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
          initialRoute: "/library",
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
                  child: child!,
                ),
              ),
            );
          },
        );
      },
    );
  }
}
