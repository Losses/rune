import 'dart:io';
import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:provider/provider.dart';
import 'package:flutter/foundation.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:intl/intl_standalone.dart';
import 'package:system_theme/system_theme.dart';
import 'package:local_notifier/local_notifier.dart';
import 'package:flutter_acrylic/flutter_acrylic.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';
import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter_fullscreen/flutter_fullscreen.dart';

import 'package:system_tray/system_tray.dart';

import 'l10n/app_localizations.dart';
import 'utils/l10n.dart';
import 'utils/locale.dart';
import 'utils/rune_log.dart';
import 'utils/platform.dart';
import 'utils/query_list.dart';
import 'utils/tray_manager.dart';
import 'utils/close_manager.dart';
import 'utils/settings_manager.dart';
import 'utils/update_color_mode.dart';
import 'utils/theme_color_manager.dart';
import 'utils/storage_key_manager.dart';
import 'utils/api/set_adaptive_switching_enabled.dart';
import 'utils/api/operate_playback_with_mix_query.dart';
import 'utils/file_storage/mac_secure_manager.dart';
import 'utils/macos_window_control_button_manager.dart';

import 'config/theme.dart';
import 'config/routes.dart';
import 'config/app_title.dart';
import 'config/shortcuts.dart';

import 'widgets/router/discovery_frame.dart';
import 'widgets/router/no_effect_page_route.dart';
import 'widgets/title_bar/window_frame.dart';
import 'widgets/shortcuts/router_actions_manager.dart';
import 'widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

import 'screens/settings_theme/constants/window_sizes.dart';

import 'bindings/bindings.dart';

import 'constants/configurations.dart';

import 'providers/crash.dart';
import 'providers/volume.dart';
import 'providers/status.dart';
import 'providers/license.dart';
import 'providers/playlist.dart';
import 'providers/scrobble.dart';
import 'providers/broadcast.dart';
import 'providers/full_screen.dart';
import 'providers/router_path.dart';
import 'providers/library_path.dart';
import 'providers/library_home.dart';
import 'providers/library_manager.dart';
import 'providers/linux_custom_window_controls.dart';
import 'providers/playback_controller.dart';
import 'providers/responsive_providers.dart';

import 'theme.dart';

late bool disableBrandingAnimation;
late String? initialPath;
late bool isWindows11;
late bool cafeMode;

void main(List<String> arguments) async {
  WidgetsFlutterBinding.ensureInitialized();
  await initializeRust(assignRustSignal);

  String? profile = arguments.contains('--profile')
      ? arguments[arguments.indexOf('--profile') + 1]
      : null;

  final SettingsManager settingsManager = SettingsManager();
  StorageKeyManager.initialize(profile);
  await MacSecureManager().completed;

  final licenseProvider = LicenseProvider();
  await licenseProvider.initialized.future;

  info$(
      'Cached license: isStoreMode: ${licenseProvider.isStoreMode}, isPro: ${licenseProvider.isPro}');

  await FullScreen.ensureInitialized();

  try {
    final DeviceInfoPlugin deviceInfo = DeviceInfoPlugin();
    final windowsInfo = await deviceInfo.windowsInfo;
    final isWindows10 = windowsInfo.productName.startsWith('Windows 10');
    isWindows11 = windowsInfo.productName.startsWith('Windows 11');

    if ((isWindows10 && appTheme.windowEffect == WindowEffect.mica) ||
        Platform.isIOS) {
      appTheme.windowEffect = WindowEffect.solid;
    }
  } catch (e) {
    info$('Device is not Windows 10, skip the patch');
  }

  final String? colorMode =
      await settingsManager.getValue<String>(kColorModeKey);

  updateColorMode(colorMode);

  await ThemeColorManager().initialize();

  final int? themeColor = await settingsManager.getValue<int>(kThemeColorKey);

  if (themeColor != null) {
    appTheme.updateThemeColor(Color(themeColor));
  }

  final String? locale = await settingsManager.getValue<String>(kLocaleKey);

  appTheme.locale = localeFromString(locale);

  disableBrandingAnimation =
      await settingsManager.getValue<bool>(kDisableBrandingAnimationKey) ??
          false;

  cafeMode = (await settingsManager.getValue<String>(kCafeModeKey)) == "true";

  // Load Linux custom window controls setting early for window configuration
  late bool linuxCustomWindowControls;
  if (Platform.isLinux) {
    linuxCustomWindowControls =
        (await settingsManager.getValue<bool>(kLinuxCustomWindowControlsKey)) ?? false;
  } else {
    linuxCustomWindowControls = false;
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
      $tray.updateTrayIcon();
    }
  };

  initialPath = await getInitialPath();
  await findSystemLocale();

  $closeManager;
  await localNotifier.setup(
    appName: 'Rune',
    shortcutPolicy: ShortcutPolicy.requireCreate,
  );

  if (isDesktop) {
    final icon = TrayManager.getTrayIcon();
    await systemTray.initSystemTray(
      title: Platform.isMacOS ? null : 'Rune',
      iconPath: icon.path,
      isTemplate: true,
      isInstalled: icon.isInstalled,
    );

    final Menu menu = Menu();
    await menu.buildFrom([
      MenuItemLabel(label: 'Show', onClicked: (menuItem) => appWindow.show()),
    ]);
    await systemTray.setContextMenu(menu);

    TrayManager.registerEventHandlers();
  }

  final windowSizeMode =
      await settingsManager.getValue<String>(kWindowSizeKey) ?? 'normal';
  final bool? rememberWindowSize =
      await SettingsManager().getValue<bool>(kRememberWindowSizeKey);

  final firstView = WidgetsBinding.instance.platformDispatcher.views.first;

  Size windowSize = windowSizes[windowSizeMode]!;

  if (rememberWindowSize == true) {
    final rememberedWindowSize = await getSavedWindowSize();

    if (rememberedWindowSize != null) {
      windowSize = rememberedWindowSize;
    }
  }

  if (isDesktop) {
    if (!Platform.isLinux) {
      appWindow.size = windowSize;
    }

    if (Platform.isLinux) {
      windowSize = windowSize / firstView.devicePixelRatio;
    }
  }

  setAdaptiveSwitchingEnabled();

  mainLoop(licenseProvider, linuxCustomWindowControls);
  if (isDesktop && !Platform.isMacOS) {
    appWindow.show();
  }

  bool? storedFullScreen =
      await settingsManager.getValue<bool>('fullscreen_state');

  if (isDesktop) {
    doWhenWindowReady(() {
      if (Platform.isMacOS) {
        MacOSWindowControlButtonManager.shared.setVertical();
      }

      appWindow.size = windowSize;
      appWindow.alignment = Alignment.center;
      appWindow.show();

      if (storedFullScreen != null) {
        FullScreen.setFullScreen(storedFullScreen);
      }

      // Initialize Linux custom window controls with the current window state
      if (Platform.isLinux) {
        // The window is now ready, we can configure any Linux-specific settings here
        // Note: Actual frameless window configuration may require platform-specific integration
      }
    });
  }
}

void mainLoop(LicenseProvider licenseProvider, bool linuxCustomWindowControls) {
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
          create: (_) => ScrobbleProvider(),
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
        ChangeNotifierProvider(create: (_) => licenseProvider),
        ChangeNotifierProvider(create: (_) => BroadcastProvider()),
        ChangeNotifierProvider(create: (_) => FullScreenProvider()),
        ChangeNotifierProvider(create: (_) => LibraryHomeProvider()),
        ChangeNotifierProvider(create: (_) => LibraryManagerProvider()),
        ChangeNotifierProvider(create: (_) => PlaybackControllerProvider()),
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => LinuxCustomWindowControlsProvider()
            ..initializeWithValue(linuxCustomWindowControls)
            ..initialize(),
        ),
      ],
      child: const Rune(),
    ),
  );
}

final rootNavigatorKey = GlobalKey<NavigatorState>();

Locale getLocaleFromPlatform() {
  final String localeName = Platform.localeName;

  final String normalized = localeName.split('.')[0];
  final List<String> parts = normalized.split('_');

  final String languageCode = parts[0];
  final String? countryCode = parts.length > 1 ? parts[1] : null;

  return Locale(languageCode, countryCode);
}

String? getWindowsFont(AppTheme theme) {
  if (!Platform.isWindows) return null;

  final locale = theme.locale ?? getLocaleFromPlatform();

  final lc = locale.languageCode.toLowerCase();
  final cc = locale.scriptCode?.toLowerCase();
  final rg = locale.countryCode?.toLowerCase();

  if (lc == 'ja') return "Yu Gothic";
  if (lc == 'ko') return "Malgun Gothic";
  if (lc == 'zh' && cc == 'hant') return "Microsoft JhengHei";
  if (lc == 'zh' && rg == 'tw') return "Microsoft JhengHei";
  if (lc == 'zh' && rg == 'hk') return "Microsoft JhengHei";
  if (lc == 'zh' && rg == 'mo') return "Microsoft JhengHei";
  if (lc == 'zh' && cc == 'hans') return "Microsoft YaHei";
  if (lc == 'zh' && rg == 'cn') return "Microsoft YaHei";
  if (lc == 'zh' && rg == 'sg') return "Microsoft YaHei";

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
          initialRoute: initialPath == null
              ? "/"
              : cafeMode
                  ? '/cover_wall'
                  : "/library",
          navigatorKey: rootNavigatorKey,
          onGenerateRoute: (settings) {
            final routeName = settings.name!;

            if (routeName == '/' || routeName == '/scanning') {
              return NoEffectPageRoute<dynamic>(
                settings: settings,
                builder: (context) => WindowFrame(
                  DiscoveryFrame(routes[routeName]!(context)),
                  customRouteName: routeName,
                ),
              );
            }

            final page = RuneWithNavigationBarAndPlaybackControllor(
              routeName: routeName,
            );

            return NoEffectPageRoute<dynamic>(
              settings: settings,
              builder: (context) => WindowFrame(DiscoveryFrame(page)),
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

class RuneLifecycleState extends State<RuneLifecycle> {
  late PlaybackStatusProvider status;
  late LicenseProvider license;
  Timer? _throttleTimer;
  bool _shouldCallUpdate = false;

  @override
  void initState() {
    super.initState();

    if (cafeMode && initialPath != null) {
      Future.delayed(
              Duration(seconds: disableBrandingAnimation == true ? 1 : 4))
          .then((_) {
        operatePlaybackWithMixQuery(
            queries: const QueryList([
              ("lib::random", "1"),
              ("filter::analyzed", "true"),
              ("pipe::recommend", "-1")
            ]),
            playbackMode: 99,
            hintPosition: -1,
            initialPlaybackId: -1,
            instantlyPlay: true,
            operateMode: PlaylistOperateMode.replace,
            fallbackPlayingItems: []);
      });
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    status = Provider.of<PlaybackStatusProvider>(context, listen: false);
    license = Provider.of<LicenseProvider>(context, listen: false);

    license.addListener(_updateLicense);
    status.addListener(_throttleUpdateTray);
    $router.addListener(_throttleUpdateTray);
    appTheme.addListener(_throttleUpdateTray);
    _throttleUpdateTray();

    _updateLicense();
  }

  @override
  dispose() {
    super.dispose();

    license.removeListener(_updateLicense);
    status.removeListener(_throttleUpdateTray);
    $router.removeListener(_throttleUpdateTray);
    appTheme.removeListener(_throttleUpdateTray);
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
    if (isDesktop) {
      $tray.updateTray(context);
    }
  }

  void _updateLicense() {
    if (isDesktop) {
      if (!license.isPro) {
        final evaluationMode = S.of(context).evaluationMode;
        appWindow.title = 'Rune [$evaluationMode]';
      } else {
        appWindow.title = 'Rune';
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return widget.child;
  }
}
