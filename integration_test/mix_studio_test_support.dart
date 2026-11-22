import 'dart:async';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rinf/rinf.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/dialogs/mix/mix_studio_dialog.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';

Future<({Directory music, LibraryRegistryProvider registry})>
openMixLibrary() async {
  final root = await Directory.systemTemp.createTemp('rune-mix-studio-');
  addTearDown(() => root.delete(recursive: true));
  await initializeRust(assignRustSignal);
  addTearDown(finalizeRust);
  await awaitResponse(
    'InitializeNodeRequest',
    InitializeNodeResponse.rustSignalStream,
    responseCallId: (signal) => signal.message.callId?.toInt(),
    send: (id) => InitializeNodeRequest(
      configPath: root.path,
      callId: id,
    ).sendSignalToRust(),
  );
  final registry = LibraryRegistryProvider();
  addTearDown(registry.dispose);
  await registry.ready;
  final music = await Directory('${root.path}/Music').create();
  final record = (await registry.registerLocalFolder(
    music.path,
    'Music',
    LibraryInitializeMode.portable,
  )).record!;
  addTearDown(() => registry.remove(record.recordId));
  await registry.open(record.recordId);
  return (music: music, registry: registry);
}

Future<Completer<Mix?>> openMixStudio(
  WidgetTester tester,
  LibraryRegistryProvider registry, {
  int? mixId,
}) async {
  final screen = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(screen, () async => null);
  addTearDown(screen.dispose);
  addTearDown(responsive.dispose);
  addTearDown(tester.view.reset);
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = const Size(900, 700);
  screen.didChangeMetrics();
  final closed = Completer<Mix?>();
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider.value(value: registry),
        ChangeNotifierProvider.value(value: screen),
        ChangeNotifierProvider.value(value: responsive),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: RevealEffectContext(
          child: MixStudioDialog(mixId: mixId, $close: closed.complete),
        ),
      ),
    ),
  );
  return closed;
}
