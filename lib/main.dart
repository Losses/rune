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
import 'package:macos_window_utils/macos_window_utils.dart';
import 'package:macos_window_utils/macos/ns_window_button_type.dart';

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

import 'config/theme.dart';
import 'config/routes.dart';
import 'config/app_title.dart';
import 'config/shortcuts.dart';

import 'widgets/window_buttons.dart';
import 'widgets/router/no_effect_page_route.dart';
import 'widgets/shortcuts/router_actions_manager.dart';
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
    WindowManipulator.overrideStandardWindowButtonPosition(
        buttonType: NSWindowButtonType.closeButton, offset: const Offset(8, 8));
    WindowManipulator.overrideStandardWindowButtonPosition(
        buttonType: NSWindowButtonType.miniaturizeButton,
        offset: const Offset(8, 28));
    WindowManipulator.overrideStandardWindowButtonPosition(
        buttonType: NSWindowButtonType.zoomButton, offset: const Offset(8, 48));
  }

  final windowSizeSetting =
      await settingsManager.getValue<String>(windowSizeKey) ?? 'normal';

  final firstView = WidgetsBinding.instance.platformDispatcher.views.first;
  final windowSize = Platform.isWindows
      ? windowSizes[windowSizeSetting]!
      : windowSizes[windowSizeSetting]! / firstView.devicePixelRatio;
  appWindow.size = windowSize;

  mainLoop();
  appWindow.show();

  doWhenWindowReady(() {
    appWindow.size = windowSize;
    appWindow.alignment = Alignment.center;
    appWindow.show();
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

            if (routeName == '/' || routeName == '/scanning') {
              return NoEffectPageRoute<dynamic>(
                settings: settings,
                builder: (context) => WindowFrame(routes[routeName]!(context)),
              );
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
            accentColor: appTheme.color,
            visualDensity: VisualDensity.standard,
          ),
          darkTheme: FluentThemeData(
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

            if (Platform.isMacOS) {
              content = MoveWindow(child: content);
            }

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
