import 'dart:io';

import 'package:rinf/rinf.dart';
import 'package:provider/provider.dart';
import 'package:flutter/foundation.dart';
import 'package:get_storage/get_storage.dart';
import 'package:system_theme/system_theme.dart';
import 'package:fluent_ui/fluent_ui.dart' hide Page;
import 'package:flutter_acrylic/flutter_acrylic.dart' as flutter_acrylic;

import 'utils/platform.dart';

import 'config/theme.dart';
import 'config/app_title.dart';
import 'config/navigation.dart';

import 'messages/generated.dart';

import 'providers/status.dart';
import 'providers/playlist.dart';
import 'providers/library_path.dart';
import 'providers/library_manager.dart';
import 'providers/transition_calculation.dart';

import 'theme.dart';
import 'router.dart';

void main() async {
  await GetStorage.init();
  await initializeRust(assignRustSignal);
  WidgetsFlutterBinding.ensureInitialized();

  // if it's not on the web, windows or android, load the accent color
  if (!kIsWeb &&
      [
        TargetPlatform.windows,
        TargetPlatform.android,
      ].contains(defaultTargetPlatform)) {
    SystemTheme.accentColor.load();
  }


  if (isDesktop && !Platform.isLinux) {
    await flutter_acrylic.Window.initialize();
  }

  runApp(
    MultiProvider(
      providers: [
        ChangeNotifierProvider(
            lazy: false, create: (_) => LibraryPathProvider()),
        ChangeNotifierProvider(create: (_) => PlaylistProvider()),
        ChangeNotifierProvider(create: (_) => PlaybackStatusProvider()),
        ChangeNotifierProvider(create: (_) => LibraryManagerProvider()),
        ChangeNotifierProvider(
            create: (_) =>
                TransitionCalculationProvider(navigationItems: navigationItems))
      ],
      child: const Rune(),
    ),
  );
}

class Rune extends StatelessWidget {
  const Rune({super.key});

  @override
  Widget build(BuildContext context) {
    PlaylistUpdateHandler.init(context);
    PlaybackStatusUpdateHandler.init(context);

    return ChangeNotifierProvider.value(
      value: appTheme,
      builder: (context, child) {
        final appTheme = context.watch<AppTheme>();

        return FluentApp.router(
          title: appTitle,
          themeMode: appTheme.mode,
          debugShowCheckedModeBanner: false,
          color: appTheme.color,
          darkTheme: FluentThemeData(
            brightness: Brightness.dark,
            accentColor: appTheme.color,
            visualDensity: VisualDensity.standard,
            focusTheme: FocusThemeData(
              glowFactor: is10footScreen(context) ? 2.0 : 0.0,
            ),
          ),
          theme: FluentThemeData(
            accentColor: appTheme.color,
            visualDensity: VisualDensity.standard,
            focusTheme: FocusThemeData(
              glowFactor: is10footScreen(context) ? 2.0 : 0.0,
            ),
          ),
          locale: appTheme.locale,
          routerDelegate: router.routerDelegate,
          routeInformationParser: router.routeInformationParser,
          routeInformationProvider: router.routeInformationProvider,
          builder: (context, child) {
            final theme = FluentTheme.of(context);

            return Container(
                color: Platform.isLinux
                    ? theme.micaBackgroundColor
                    : Colors.transparent,
                child: Directionality(
                  textDirection: appTheme.textDirection,
                  child: child!,
                ));
          },
        );
      },
    );
  }
}
