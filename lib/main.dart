import 'dart:io';
import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:provider/provider.dart';
import 'package:flutter/foundation.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:intl/intl_standalone.dart';
import 'package:tray_manager/tray_manager.dart';
import 'package:system_theme/system_theme.dart';
import 'package:flutter_acrylic/flutter_acrylic.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';
import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter_fullscreen/flutter_fullscreen.dart';

import 'package:flutter_gen/gen_l10n/app_localizations.dart';

import 'utils/locale.dart';
import 'utils/platform.dart';
import 'utils/rune_log.dart';
import 'utils/tray_manager.dart';
import 'utils/api/play_next.dart';
import 'utils/api/play_play.dart';
import 'utils/close_manager.dart';
import 'utils/api/play_pause.dart';
import 'utils/settings_manager.dart';
import 'utils/api/play_previous.dart';
import 'utils/update_color_mode.dart';
import 'utils/theme_color_manager.dart';
import 'utils/storage_key_manager.dart';
import 'utils/file_storage/mac_secure_manager.dart';
import 'utils/macos_window_control_button_manager.dart';

import 'config/theme.dart';
import 'config/routes.dart';
import 'config/app_title.dart';
import 'config/shortcuts.dart';

import 'widgets/router/no_effect_page_route.dart';
import 'widgets/title_bar/window_frame.dart';
import 'widgets/shortcuts/router_actions_manager.dart';
import 'widgets/navigation_bar/utils/macos_move_window.dart';
import 'widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

import 'screens/settings_theme/settings_theme.dart';
import 'screens/settings_theme/constants/window_sizes.dart';
import 'screens/settings_language/settings_language.dart';

import 'messages/generated.dart';

import 'providers/crash.dart';
import 'providers/volume.dart';
import 'providers/status.dart';
import 'providers/playlist.dart';
import 'providers/full_screen.dart';
import 'providers/router_path.dart';
import 'providers/library_path.dart';
import 'providers/library_home.dart';
import 'providers/library_manager.dart';
import 'providers/playback_controller.dart';
import 'providers/responsive_providers.dart';

import 'theme.dart';

late bool disableBrandingAnimation;
late String? initialPath;
late bool isWindows11;
late bool isPro;

void main(List<String> arguments) async {
  WidgetsFlutterBinding.ensureInitialized();

  isPro = arguments.contains('--pro');

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
    isWindows11 = windowsInfo.productName.startsWith('Windows 11');

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

  final String? locale = await settingsManager.getValue<String>(localeKey);

  appTheme.locale = localeFromString(locale);

  disableBrandingAnimation =
      await settingsManager.getValue<bool>(disableBrandingAnimationKey) ??
          false;

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
    appTheme.addListener(updateTheme);
    updateTheme();
  }

  WidgetsBinding.instance.platformDispatcher.onPlatformBrightnessChanged = () {
    WidgetsBinding.instance.handlePlatformBrightnessChanged();
    if (Platform.isMacOS || Platform.isWindows) {
      updateTheme();
    }

    if (Platform.isWindows) {
      $tray.initialize();
    }
  };

  initialPath = await getInitialPath();
  await findSystemLocale();

  await $tray.initialize();

  $closeManager;

  if (Platform.isMacOS) {
    MacOSWindowControlButtonManager.setVertical();
  }

  final windowSizeSetting =
      await settingsManager.getValue<String>(windowSizeKey) ?? 'normal';

  final firstView = WidgetsBinding.instance.platformDispatcher.views.first;
  final windowSize = Platform.isWindows || Platform.isMacOS
      ? windowSizes[windowSizeSetting]!
      : windowSizes[windowSizeSetting]! / firstView.devicePixelRatio;
  appWindow.size = windowSize;

  mainLoop();
  appWindow.show();

  bool? storedFullScreen =
      await settingsManager.getValue<bool>('fullscreen_state');

  doWhenWindowReady(() {
    appWindow.size = windowSize;
    appWindow.alignment = Alignment.center;
    appWindow.show();

    if (storedFullScreen != null) {
      FullScreen.setFullScreen(storedFullScreen);
    }
  });
}

void mainLoop() {
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
        ChangeNotifierProvider(create: (_) => LibraryHomeProvider()),
        ChangeNotifierProvider(create: (_) => PlaybackControllerProvider()),
        ChangeNotifierProvider(create: (_) => LibraryManagerProvider()),
        ChangeNotifierProvider(create: (_) => FullScreenProvider()),
      ],
      child: const Rune(),
    ),
  );
}

final rootNavigatorKey = GlobalKey<NavigatorState>();

String? getWindowsFont(AppTheme theme) {
  if (!Platform.isWindows) return null;

  final lc = theme.locale?.languageCode.toLowerCase();
  final cc = theme.locale?.scriptCode?.toLowerCase();

  if (lc == 'ja') return "Yu Gothic";
  if (lc == 'ko') return 'Malgun Gothic';
  if (lc == 'zh' && cc == 'hant') return "Microsoft JhengHei";
  if (lc == 'zh' && cc == 'hans') return "Microsoft YaHei";

  return null;
}

class Rune extends StatefulWidget {
  const Rune({super.key});

  @override
  State<Rune> createState() => _RuneState();
}

class _RuneState extends State<Rune> {
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

            if (!Platform.isMacOS) {
              if (routeName == '/' || routeName == '/scanning') {
                return NoEffectPageRoute<dynamic>(
                  settings: settings,
                  builder: (context) =>
                      WindowFrame(routes[routeName]!(context)),
                );
              }
            } else {
              if (routeName == '/scanning') {
                return NoEffectPageRoute<dynamic>(
                  settings: settings,
                  builder: (context) => MacOSMoveWindow(
                    child: WindowFrame(routes[routeName]!(context)),
                  ),
                );
              }
              if (routeName == '/') {
                return NoEffectPageRoute<dynamic>(
                  settings: settings,
                  builder: (context) => MacOSMoveWindow(
                    isEnabledDoubleTap: false,
                    child: WindowFrame(routes[routeName]!(context)),
                  ),
                );
              }
            }

            final page = RuneWithNavigationBarAndPlaybackControllor(
              routeName: routeName,
            );

            return NoEffectPageRoute<dynamic>(
              settings: settings,
              builder: (context) => WindowFrame(page),
            );
          },
          debugShowCheckedModeBanner: false,
          color: appTheme.color,
          themeMode: appTheme.mode,
          theme: FluentThemeData(
            fontFamily: getWindowsFont(appTheme),
            accentColor: appTheme.color,
            visualDensity: VisualDensity.standard,
          ),
          darkTheme: FluentThemeData(
            fontFamily: getWindowsFont(appTheme),
            brightness: Brightness.dark,
            accentColor: appTheme.color,
            visualDensity: VisualDensity.standard,
          ),
          locale: appTheme.locale,
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          builder: (context, child) {
            final theme = FluentTheme.of(context);

            Widget content = Container(
              color: appTheme.windowEffect == WindowEffect.solid
                  ? theme.micaBackgroundColor
                  : Colors.transparent,
              child: Directionality(
                textDirection: appTheme.textDirection,
                child: Shortcuts(
                  shortcuts: shortcuts,
                  child: NavigationShortcutManager(
                    RuneLifecycle(
                      RevealEffectContext(child: child!),
                    ),
                  ),
                ),
              ),
            );

            return content;
          },
        );
      },
    );
  }
}

class RuneLifecycle extends StatefulWidget {
  final Widget child;
  const RuneLifecycle(this.child, {super.key});

  @override
  RuneLifecycleState createState() => RuneLifecycleState();
}

class RuneLifecycleState extends State<RuneLifecycle> with TrayListener {
  late PlaybackStatusProvider status;
  Timer? _throttleTimer;
  bool _shouldCallUpdate = false;

  @override
  void initState() {
    super.initState();
    trayManager.addListener(this);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    status = Provider.of<PlaybackStatusProvider>(context, listen: false);

    status.addListener(_throttleUpdateTray);
    $router.addListener(_throttleUpdateTray);
    appTheme.addListener(_throttleUpdateTray);
  }

  @override
  dispose() {
    super.dispose();

    trayManager.removeListener(this);
    appTheme.removeListener(_throttleUpdateTray);
    status.removeListener(_throttleUpdateTray);
    $router.removeListener(_throttleUpdateTray);
    _throttleTimer?.cancel();
  }

  void _throttleUpdateTray() {
    if (_throttleTimer?.isActive ?? false) {
      _shouldCallUpdate = true;
    } else {
      _updateTray();
      _throttleTimer = Timer(const Duration(milliseconds: 500), () {
        if (_shouldCallUpdate) {
          _updateTray();
          _shouldCallUpdate = false;
        }
      });
    }
  }

  void _updateTray() {
    $tray.updateTray(context);
  }

  @override
  void onTrayMenuItemClick(MenuItem menuItem) async {
    if (menuItem.key == 'show_window') {
      appWindow.show();
    } else if (menuItem.key == 'exit_app') {
      $closeManager.close();
    } else if (menuItem.key == 'previous') {
      playPrevious();
    } else if (menuItem.key == 'play') {
      playPlay();
    } else if (menuItem.key == 'pause') {
      playPause();
    } else if (menuItem.key == 'next') {
      playNext();
    }
  }

  @override
  void onTrayIconMouseDown() {
    if (Platform.isWindows) {
      appWindow.show();
    } else {
      trayManager.popUpContextMenu();
    }
  }

  @override
  void onTrayIconRightMouseDown() {
    trayManager.popUpContextMenu();
  }

  @override
  Widget build(BuildContext context) {
    return widget.child;
  }
}
