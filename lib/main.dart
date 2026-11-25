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
import 'package:macos_window_utils/macos_window_utils.dart'
    show WindowManipulator;
import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter_fullscreen/flutter_fullscreen.dart';

import 'package:system_tray/system_tray.dart';

import 'l10n/app_localizations.dart';
import 'utils/blink_cursor_option.dart';
import 'utils/profile_option.dart';
import 'utils/display_blank_option.dart';
import 'utils/l10n.dart';
import 'utils/api/close_library.dart';
import 'utils/api/request_failure.dart';
import 'utils/dialogs/failed_to_initialize_library.dart';
import 'utils/remote_connection_messages.dart';
import 'utils/share_sync_messages.dart';
import 'utils/router/navigation.dart';
import 'utils/router/router_aware_flyout_controller.dart';
import 'utils/locale.dart';
import 'utils/rune_log.dart';
import 'utils/scene_files.dart';
import 'utils/platform.dart';
import 'utils/query_list.dart';
import 'utils/tray_manager.dart';
import 'utils/close_manager.dart';
import 'utils/settings_manager.dart';
import 'utils/update_color_mode.dart';
import 'utils/theme_color_manager.dart';
import 'utils/storage_key_manager.dart';
import 'utils/api/set_adaptive_switching_enabled.dart';
import 'utils/api/set_mix_with_others.dart';
import 'utils/api/operate_playback_with_mix_query.dart';
import 'utils/start_cafe_playback_after_open.dart';
import 'utils/recover_failed_startup_open.dart';
import 'utils/file_storage/mac_secure_manager.dart';
import 'utils/macos_window_control_button_manager.dart';

import 'config/theme.dart';
import 'config/routes.dart';
import 'config/app_title.dart';
import 'config/shortcuts.dart';

import 'widgets/app_viewport.dart';
import 'widgets/app_scale_shortcuts.dart';
import 'widgets/status_bar_style.dart';
import 'widgets/dismiss_keyboard_on_tap.dart';
import 'widgets/router/discovery_frame.dart';
import 'widgets/router/no_effect_page_route.dart';
import 'widgets/title_bar/window_frame.dart';
import 'widgets/shortcuts/router_actions_manager.dart';
import 'widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

import 'screens/settings_theme/constants/window_sizes.dart';

import 'bindings/bindings.dart';

import 'constants/configurations.dart';

import 'providers/app_scale.dart';
import 'providers/crash.dart';
import 'providers/remote_connection_loss.dart';
import 'providers/visualizer_scenes.dart';
import 'providers/visualizer_lab.dart';
import 'providers/volume.dart';
import 'providers/status.dart';
import 'providers/license.dart';
import 'providers/playlist.dart';
import 'providers/scrobble.dart';
import 'providers/broadcast.dart';
import 'providers/full_screen.dart';
import 'providers/router_path.dart';
import 'providers/library_open_state.dart';
import 'providers/library_registry.dart';
import 'utils/dialogs/subsonic/subsonic_relogin_dialog.dart';
import 'utils/hosted_player/hosted_player.dart';
import 'providers/library_home.dart';
import 'providers/library_manager.dart';
import 'providers/local_folder_picker.dart';
import 'providers/linux_custom_window_controls.dart';
import 'providers/playback_controller.dart';
import 'providers/controllers.dart';
import 'providers/renderer_attachment.dart';
import 'providers/request_capabilities.dart';
import 'providers/playing_item_capabilities.dart';
import 'providers/renderer_link_loss.dart';
import 'providers/library_share_sync.dart';
import 'providers/remote_library_reconnect.dart';
import 'providers/responsive_providers.dart';

import 'theme.dart';

late bool disableBrandingAnimation;
late String? initialPath;
late final LibraryRegistryProvider libraryRegistry;
// Whether startup opens a library: the development LIBRARY_PATH or the record open last.
late bool hasStartupLibrary;
bool isWindows11 = false;
late bool cafeMode;

void main(List<String> arguments) async {
  WidgetsFlutterBinding.ensureInitialized();
  PlatformDispatcher.instance.onError = (error, stack) {
    if (error is! RequestFailure) return false;
    if (error.needsServiceSignIn) return true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final context = _noticeContext();
      if (context != null && context.mounted) {
        showRequestFailureInfoBar(context, error);
      }
    });
    WidgetsBinding.instance.ensureVisualUpdate();
    return true;
  };
  if (Platform.isLinux) {
    try {
      await AcrylicShaderWarmUp(
        devicePixelRatio:
            WidgetsBinding
                .instance
                .platformDispatcher
                .implicitView
                ?.devicePixelRatio ??
            1.0,
        scaleFactors: AppScaleProvider.supportedPercentages
            .map((percentage) => percentage / 100)
            .toList(growable: false),
      ).execute();
    } catch (error) {
      warn$('Acrylic shader warm-up failed: $error');
    }
  }
  await initializeRust(assignRustSignal);
  // Start device-level sharing and sync before any library is open. A
  // failure here is not fatal: opening a library retries and reports it.
  try {
    final configPath = await getSettingsPath();
    await awaitResponse(
      'InitializeNodeRequest',
      InitializeNodeResponse.rustSignalStream,
      responseCallId: (response) => response.message.callId?.toInt(),
      send: (callId) => InitializeNodeRequest(
        configPath: configPath,
        callId: callId,
      ).sendSignalToRust(),
    );
  } on TimeoutException {
    error$('Device initialization did not answer within 30 seconds');
  } on RequestFailure catch (failure) {
    error$('Could not initialize this device: ${failure.detail}');
  }
  // Scene files the app was opened with, or is asked to open later.
  SceneFiles.start(arguments);

  appTheme.cursorBlinks = cursorBlinksAtLaunch(
    arguments,
    kIsWeb ? const {} : Platform.environment,
  );

  // The kiosk's flutter-pi switches the display off after a time without
  // input, and tells Flutter the app is paused while it is off.
  final blankAfter = blankAfterAtLaunch(
    kIsWeb ? const {} : Platform.environment,
  );
  if (blankAfter != null) {
    await DisplayBlanker.setBlankAfter(blankAfter);
  }

  final SettingsManager settingsManager = SettingsManager();
  StorageKeyManager.initialize(profileAtLaunch(arguments));
  await detectIosDevice();
  final appScale = AppScaleProvider(
    supportsFollowTextSize: iosDevice == IosDevice.iPhone,
  );
  await appScale.initialize();
  await MacSecureManager().completed;

  final licenseProvider = LicenseProvider();
  await licenseProvider.initialized.future;

  info$(
    'Cached license: isStoreMode: ${licenseProvider.isStoreMode}, isPro: ${licenseProvider.isPro}',
  );

  await FullScreen.ensureInitialized();

  // Product names can still report Windows 10 on Windows 11.
  if (Platform.isWindows) {
    try {
      final windowsInfo = await DeviceInfoPlugin().windowsInfo;
      isWindows11 = windowsInfo.buildNumber >= 22000;
    } catch (error) {
      warn$('Could not detect Windows backdrop support: $error');
    }

    // Use a painted background when Mica support cannot be established.
    if (!isWindows11) {
      appTheme.windowEffect = WindowEffect.solid;
    }
  }

  final String? colorMode = await settingsManager.getValue<String>(
    kColorModeKey,
  );

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
        (await settingsManager.getValue<bool>(kLinuxCustomWindowControlsKey)) ??
        false;
  } else {
    linuxCustomWindowControls = false;
  }

  // Resolve the system accent before building the first theme.
  if (!kIsWeb &&
      [
        TargetPlatform.windows,
        TargetPlatform.android,
      ].contains(defaultTargetPlatform)) {
    await SystemTheme.accentColor.load();
  }

  if (isDesktop && !Platform.isLinux) {
    try {
      await Window.initialize();
      if (Platform.isMacOS) {
        // Window.initialize() turns the full-size content view off, and bitsdojo_window turns it
        // back on when it shows the window. Turning it on before the first frame keeps the
        // content one size while the window first shows: a change there can make Flutter's macOS
        // compositor hand Impeller a surface of the old size, which crashes the raster thread.
        await WindowManipulator.enableFullSizeContentView();
      }
      await appTheme.setEffect();
    } catch (error) {
      warn$('Window effect initialization failed: $error');
      appTheme.windowEffect = WindowEffect.solid;
    }

    // Later theme changes must not depend on another frame being scheduled.
    appTheme.addListener(updateTheme);
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
  libraryRegistry = LibraryRegistryProvider();
  try {
    await libraryRegistry.ready;
  } on Object catch (error) {
    error$('Could not list the libraries: $error');
  }
  hasStartupLibrary = shouldOpenLibraryAtStartup(initialPath, libraryRegistry);
  await findSystemLocale();

  $closeManager;
  await localNotifier.setup(
    appName: 'Rune',
    shortcutPolicy: ShortcutPolicy.requireCreate,
  );

  if (isDesktop) {
    final icon = await TrayManager.getTrayIcon();
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
  final bool? rememberWindowSize = await SettingsManager().getValue<bool>(
    kRememberWindowSizeKey,
  );

  final firstView = WidgetsBinding.instance.platformDispatcher.views.first;

  Size windowSize = windowSizes[windowSizeMode]!;
  var hasRememberedSize = false;

  if (rememberWindowSize == true) {
    final rememberedWindowSize = await getSavedWindowSize();

    if (rememberedWindowSize != null) {
      windowSize = rememberedWindowSize;
      hasRememberedSize = true;
    }
  }

  if (isDesktop) {
    if (!Platform.isLinux) {
      appWindow.size = windowSize;
    }

    if (Platform.isLinux && !hasRememberedSize) {
      windowSize = windowSize / firstView.devicePixelRatio;
    }
  }

  setAdaptiveSwitchingEnabled();
  setMixWithOthers();

  mainLoop(licenseProvider, linuxCustomWindowControls, appScale);
  if (isDesktop && !Platform.isMacOS) {
    appWindow.show();
  }

  bool? storedFullScreen = await settingsManager.getValue<bool>(
    'fullscreen_state',
  );

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

void mainLoop(
  LicenseProvider licenseProvider,
  bool linuxCustomWindowControls,
  AppScaleProvider appScale,
) {
  runApp(
    MultiProvider(
      providers: [
        ChangeNotifierProvider.value(value: appScale),
        ChangeNotifierProvider(lazy: false, create: (_) => CrashProvider()),
        ChangeNotifierProvider(lazy: false, create: (_) => ScrobbleProvider()),
        ChangeNotifierProvider(lazy: false, create: (_) => PlaylistProvider()),
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => RendererAttachmentProvider(),
        ),
        // After the renderer attachment: the system volume stands in only without a renderer.
        ChangeNotifierProvider(
          lazy: false,
          create: (context) => VolumeProvider(
            renderers: context.read<RendererAttachmentProvider>(),
          ),
        ),
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => RequestCapabilitiesProvider(),
        ),
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => PlayingItemCapabilitiesProvider(),
        ),
        ChangeNotifierProvider(
          lazy: false,
          create: (_) => ControllersProvider(),
        ),
        ChangeNotifierProvider(
          lazy: false,
          create: (context) =>
              ScreenSizeProvider(context.read<AppScaleProvider>()),
        ),
        ChangeNotifierProvider(
          lazy: false,
          // The theme follows the cover only of an item whose library gives
          // its color.
          create: (context) {
            final capabilities = context.read<PlayingItemCapabilitiesProvider>();
            return PlaybackStatusProvider(
              colorCapability: (item) => capabilities.capabilityFor(
                item,
                (c) => c.getPrimaryColorByTrackIdRequest,
              ),
            );
          },
        ),
        ChangeNotifierProvider.value(value: libraryRegistry),
        Provider(create: (_) => LocalFolderPickerProvider()),
        ChangeNotifierProvider(
          lazy: false,
          create: (context) => LibraryOpenStateProvider(
            initialPath,
            context.read<LibraryRegistryProvider>(),
          ),
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
        ChangeNotifierProvider(create: (_) => VisualizerScenesProvider()),
        ChangeNotifierProxyProvider2<
          PlaybackStatusProvider,
          VisualizerScenesProvider,
          VisualizerLabProvider
        >(
          lazy: false,
          create: (_) => VisualizerLabProvider(),
          update: (_, status, scenes, previous) => previous!
            ..attachStatus(status)
            ..attachGpu(scenes.gpu),
        ),
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

/// A context inside the root navigator's overlay, for the notices shown over
/// every page. [Overlay.of] finds an overlay only from inside one of its
/// entries, so the overlay's own context will not do. The context belongs to
/// the bottom page, which can close while the notice is up, so a notice
/// builds from its own context instead of this one.
BuildContext? _noticeContext() {
  final overlay = rootNavigatorKey.currentState?.overlay;
  var context = overlay?.context;
  while (context != null && Overlay.maybeOf(context) != overlay) {
    BuildContext? child;
    context.visitChildElements((element) => child ??= element);
    context = child;
  }
  return context;
}

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
  /// Hands the menus shown on the app's own navigator their routes.
  final _flyoutRoutes = FlyoutRouteObserver();

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider.value(
      value: appTheme,
      builder: (context, child) {
        final appTheme = context.watch<AppTheme>();

        return FluentApp(
          title: appTitle,
          initialRoute: !hasStartupLibrary
              ? "/"
              : cafeMode
              ? '/cover_wall'
              : "/library",
          navigatorKey: rootNavigatorKey,
          navigatorObservers: [_flyoutRoutes],
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
          theme: appTheme.themeData(
            Brightness.light,
            fontFamily: getWindowsFont(appTheme),
          ),
          darkTheme: appTheme.themeData(
            Brightness.dark,
            fontFamily: getWindowsFont(appTheme),
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
                  child: DismissKeyboardOnTap(
                    child: NavigationShortcutManager(
                      RuneLifecycle(RevealEffectContext(child: child!)),
                    ),
                  ),
                ),
              ),
            );

            return StatusBarStyle(
              child: AppViewport(child: AppScaleShortcuts(child: content)),
            );
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
  late final RemoteConnectionLossHandler _remoteLoss;
  late final RendererLinkLossHandler _rendererLinkLoss;
  late final LibraryShareSyncNotice _shareSyncNotice;
  late final RemoteLibraryReconnectNotice _remoteReconnect;
  late final HostedPlayerBridge _hostedPlayer;
  LibraryOpenStateProvider? _libraryState;
  String? _noticeOpenRecordId;
  StreamSubscription<RustSignalPack<RequestFailedResponse>>?
  _requestFailureSubscription;
  bool _subsonicReloginOpen = false;
  bool _startupOpenChecked = false;

  @override
  void initState() {
    super.initState();

    final registry = context.read<LibraryRegistryProvider>();
    _requestFailureSubscription = RequestFailedResponse.rustSignalStream.listen(
      (signal) {
        final failure = signal.message;
        if (failure.cause
            case RequestFailureCauseServiceSignInRequired(:final recordId)) {
          if (_subsonicReloginOpen) return;
          final dialogContext = rootNavigatorKey.currentState?.overlay?.context;
          if (!signsInWithKeptCredential(registry.records, recordId) ||
              dialogContext == null ||
              !mounted ||
              !dialogContext.mounted) {
            return;
          }
          _subsonicReloginOpen = true;
          showSubsonicReloginDialog(
            dialogContext,
            registry,
            recordId,
          ).whenComplete(() => _subsonicReloginOpen = false);
          return;
        }
        if (failure.callId != null) {
          return;
        }
        final noticeContext = _noticeContext();
        if (noticeContext == null || !mounted || !noticeContext.mounted) return;
        showRequestFailureInfoBar(
          noticeContext,
          RequestFailure(
            failure.request,
            failure.kind,
            failure.detail,
            failure.cause,
          ),
        );
      },
    );

    _hostedPlayer = HostedPlayerBridge.attached();

    _remoteReconnect = RemoteLibraryReconnectNotice(
      reconnecting: RemoteLibraryReconnecting.rustSignalStream.map(
        (signal) => signal.message,
      ),
      reconnected: RemoteLibraryReconnected.rustSignalStream.map(
        (signal) => signal.message,
      ),
      losses: RemoteConnectionLost.rustSignalStream.map(
        (signal) => signal.message,
      ),
    );
    _remoteLoss = RemoteConnectionLossHandler(
      losses: RemoteConnectionLost.rustSignalStream.map(
        (signal) => signal.message,
      ),
      closeLibrary: () => closeLibrary(context),
      showLoss: (kind) {
        if (!mounted) return;
        final message = remoteLossMessage(S.of(context), kind);
        $$replace('/');
        WidgetsBinding.instance.addPostFrameCallback((_) {
          final rootContext = rootNavigatorKey.currentContext;
          if (rootContext != null) {
            showFailedToInitializeLibrary(rootContext, message);
          }
        });
      },
    );
    _shareSyncNotice = LibraryShareSyncNotice(
      updates: LibrarySyncStatusUpdated.rustSignalStream.map(
        (signal) => signal.message,
      ),
      showNotice: (failure) {
        final context = _noticeContext();
        if (context == null || !context.mounted) return;
        final message = shareSyncFailureMessage(S.of(context), failure);
        displayInfoBar(
          context,
          builder: (_, close) => InfoBar(
            title: Text(message),
            severity: InfoBarSeverity.warning,
            onClose: close,
          ),
        );
      },
    );
    _rendererLinkLoss = RendererLinkLossHandler(
      losses: RendererLinkLost.rustSignalStream.map((signal) => signal.message),
      showNotice: (alias) {
        final context = _noticeContext();
        if (context == null || !context.mounted) return;
        displayInfoBar(
          context,
          builder: (context, close) => InfoBar(
            title: Text(S.of(context).rendererLinkLost(alias)),
            severity: InfoBarSeverity.warning,
            onClose: close,
          ),
        );
      },
    );

    if (cafeMode && hasStartupLibrary) {
      unawaited(
        startCafePlaybackAfterOpen(
          libraryOpened: context.read<LibraryOpenStateProvider>().initialOpen,
          play: () async {
            // A library without analysis data cannot roam.
            if (!mounted || !canStartRoaming(context)) return;
            await operatePlaybackWithMixQuery(
              queries: const QueryList([
                ("lib::random", "1"),
                ("filter::analyzed", "true"),
                ("pipe::recommend", "-1"),
              ]),
              playbackMode: null,
              hintPosition: -1,
              initialPlaybackId: null,
              instantlyPlay: true,
              operateMode: PlaylistOperateMode.replace,
              fallbackPlayingItems: [],
            );
          },
        ),
      );
    }
  }

  Future<void> _recoverFailedStartupOpen(LibraryOpenStateProvider libraryPath) {
    final registry = libraryPath.registry;
    return recoverFailedStartupOpen(
      libraryOpened: libraryPath.initialOpen,
      failure: () => libraryPath.initialOpenFailure,
      libraryIsOpen: () => libraryPath.hasOpenLibrary,
      signInAgain: () async {
        await WidgetsBinding.instance.endOfFrame;
        final recordId = registry.lastOpenRecordId;
        final dialogContext = rootNavigatorKey.currentState?.overlay?.context;
        if (recordId == null ||
            dialogContext == null ||
            !dialogContext.mounted) {
          return false;
        }
        final updated = await showSubsonicReloginDialog(
          dialogContext,
          registry,
          recordId,
        );
        if (!updated) return false;
        await registry.open(recordId);
        return true;
      },
      showChooser: (failure) {
        if (!mounted) return;
        final message = failure == null
            ? null
            : libraryFailureMessage(S.of(context), failure);
        $$replace('/');
        WidgetsBinding.instance.addPostFrameCallback((_) {
          final rootContext = rootNavigatorKey.currentContext;
          if (rootContext != null) {
            showFailedToInitializeLibrary(rootContext, message);
          }
        });
      },
    );
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    final libraryPath = context.read<LibraryOpenStateProvider>();
    if (!_startupOpenChecked) {
      _startupOpenChecked = true;
      if (hasStartupLibrary) {
        unawaited(_recoverFailedStartupOpen(libraryPath));
      }
    }
    if (_libraryState != libraryPath) {
      _libraryState?.removeListener(_libraryChanged);
      _libraryState = libraryPath;
      _noticeOpenRecordId = libraryPath.registry.openRecordId;
      libraryPath.addListener(_libraryChanged);
    }

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

    _remoteLoss.dispose();
    _rendererLinkLoss.dispose();
    _shareSyncNotice.dispose();
    _hostedPlayer.dispose();
    _requestFailureSubscription?.cancel();
    _libraryState?.removeListener(_libraryChanged);
    _remoteReconnect.dispose();
    license.removeListener(_updateLicense);
    status.removeListener(_throttleUpdateTray);
    $router.removeListener(_throttleUpdateTray);
    appTheme.removeListener(_throttleUpdateTray);
    _throttleTimer?.cancel();
  }

  void _libraryChanged() {
    final recordId = _libraryState?.registry.openRecordId;
    if (recordId != _noticeOpenRecordId) {
      _noticeOpenRecordId = recordId;
      _remoteReconnect.clear();
    }
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
    return ValueListenableBuilder<String?>(
      valueListenable: _remoteReconnect,
      child: widget.child,
      builder: (context, alias, child) => Stack(
        fit: StackFit.expand,
        children: [
          child!,
          if (alias != null)
            Positioned(
              left: 16,
              right: 16,
              bottom: 24,
              child: SafeArea(
                child: InfoBar(
                  title: Text(S.of(context).remoteLibraryReconnecting(alias)),
                  severity: InfoBarSeverity.info,
                ),
              ),
            ),
        ],
      ),
    );
  }
}
