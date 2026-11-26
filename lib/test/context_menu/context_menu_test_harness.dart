import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/library_open_state.dart';
import '../../providers/library_registry.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/responsive_providers.dart';
import '../../utils/api/request_failure.dart';

/// A Subsonic library, which keeps no mixes.
const subsonicRecord = LibraryRecordSummary(
  shareSync: false,
  recordId: 'subsonic-record',
  alias: 'Server',
  kind: LibraryRecordKindSummarySubsonic(
    endpoint: 'http://127.0.0.1:4533',
    account: 'listener',
  ),
);

/// A folder library, which keeps mixes.
const folderRecord = LibraryRecordSummary(
  shareSync: false,
  recordId: 'folder-record',
  alias: 'Music',
  kind: LibraryRecordKindSummaryLocal(libraryId: 'library-1', path: '/music'),
);

/// The answer a Subsonic library gives to the mix requests, which it does not
/// implement.
const mixesUnsupported = RequestFailure(
  'FetchAllMixesRequest',
  RequestErrorKind.unavailable,
  'FetchAllMixesRequest is unsupported by this target: not implemented',
);

/// The capabilities of a library that keeps no mixes, such as a Subsonic one.
class NoMixCapabilities extends RequestCapabilitiesProvider {
  NoMixCapabilities() : super(changes: const Stream.empty());

  @override
  RequestCapability get fetchAllMixesRequest => const RequestCapabilityHidden();

  @override
  RequestCapability get addItemToMixRequest => const RequestCapabilityHidden();
}

/// Opens [record] as the library of a desktop window that shows [target], then
/// right-clicks the widget keyed [targetKey]. Without [capabilities] the window
/// has no capabilities provider, so every request counts as available.
Future<void> openContextMenu(
  WidgetTester tester, {
  required LibraryRecordSummary record,
  RequestCapabilitiesProvider? capabilities,
  required Widget target,
  required Key targetKey,
}) async {
  // A desktop window, so the menu loads its data before it opens.
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = const Size(1920, 1080);
  addTearDown(tester.view.reset);
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  addTearDown(sizes.dispose);
  addTearDown(responsive.dispose);
  final registry = LibraryRegistryProvider(
    loadRecords: () async => ListLibraryRecordsResponse(
      records: [record],
      invalidRecords: const [],
      open: OpenRecord(
        recordId: record.recordId,
        generation: Uint64.fromBigInt(BigInt.one),
      ),
      lastOpenRecord: null,
    ),
  );
  addTearDown(registry.dispose);
  final openState = LibraryOpenStateProvider(null, registry);
  addTearDown(openState.dispose);
  await openState.initialOpen;
  if (capabilities != null) addTearDown(capabilities.dispose);

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
        ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
        ChangeNotifierProvider<LibraryOpenStateProvider>.value(
          value: openState,
        ),
        if (capabilities != null)
          ChangeNotifierProvider<RequestCapabilitiesProvider>.value(
            value: capabilities,
          ),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Center(child: target),
      ),
    ),
  );
  // Lets the screen size provider save the window size it read.
  await tester.pump(const Duration(seconds: 1));

  final gesture = await tester.startGesture(
    tester.getCenter(find.byKey(targetKey)),
    kind: PointerDeviceKind.mouse,
    buttons: kSecondaryMouseButton,
  );
  await gesture.up();
  await tester.pumpAndSettle();
}
