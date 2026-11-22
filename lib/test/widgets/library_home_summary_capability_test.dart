import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/library_home.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/responsive_providers.dart';
import '../../screens/library_home/large_screen_library_home_list.dart';
import '../../utils/api/request_failure.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';

/// The answer a Subsonic library gives to the summary request, which it does
/// not implement.
const _summaryUnsupported = RequestFailure(
  'ComplexQueryRequest',
  RequestErrorKind.unavailable,
  'ComplexQueryRequest is unsupported by this target: not implemented',
);

/// The capabilities of a library that offers no summaries, such as a Subsonic
/// one.
class _NoSummaryCapabilities extends RequestCapabilitiesProvider {
  _NoSummaryCapabilities() : super(changes: const Stream.empty());

  @override
  RequestCapability get complexQueryRequest => const RequestCapabilityHidden();
}

/// Shows the large-screen library home with its default entries, and waits
/// until its summary is settled and its tiles have animated in.
Future<void> _pumpHome(
  WidgetTester tester, {
  required Future<ComplexQueryResponse?> Function(List<ComplexQuery> queries)
  loadSummary,
  RequestCapabilitiesProvider? capabilities,
}) async {
  final home = LibraryHomeProvider(readStored: () async => null);
  addTearDown(home.dispose);
  final layout = StartScreenLayoutManager();
  addTearDown(layout.dispose);

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<StartScreenLayoutManager>.value(value: layout),
        ChangeNotifierProvider<LibraryHomeProvider>.value(value: home),
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
        ),
        ChangeNotifierProvider(
          create: (context) => ResponsiveProvider(
            context.read<ScreenSizeProvider>(),
            () async => null,
          ),
        ),
        if (capabilities != null)
          ChangeNotifierProvider<RequestCapabilitiesProvider>.value(
            value: capabilities,
          ),
      ],
      child: FluentApp(
        locale: const Locale('en'),
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: ScaffoldPage(
          content: RevealEffectContext(
            child: PageContentFrame(
              child: LargeScreenLibraryHomeListView(
                layoutManager: layout,
                loadSummary: loadSummary,
              ),
            ),
          ),
        ),
      ),
    ),
  );
  await tester.pump();
  expect(home.loaded, isTrue);
  // Covers the animation delay and the tiles' staggered entrance.
  for (var frame = 0; frame < 20; frame++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

void main() {
  testWidgets('a library home without summaries still offers its navigation', (
    tester,
  ) async {
    final capabilities = _NoSummaryCapabilities();
    addTearDown(capabilities.dispose);
    var summaryRequests = 0;

    await _pumpHome(
      tester,
      capabilities: capabilities,
      loadSummary: (queries) async {
        summaryRequests++;
        throw _summaryUnsupported;
      },
    );

    expect(find.text('Tracks'), findsOneWidget);
    expect(find.text('This is not available right now.'), findsNothing);
    expect(find.text('No data available'), findsNothing);
    expect(summaryRequests, 0);
  });

  testWidgets('a library home with summaries still asks for them', (
    tester,
  ) async {
    var summaryRequests = 0;

    await _pumpHome(
      tester,
      loadSummary: (queries) async {
        summaryRequests++;
        return const ComplexQueryResponse(result: []);
      },
    );

    expect(summaryRequests, 1);
    expect(find.text('No data available'), findsOneWidget);
  });

  testWidgets('a library home shows its summaries beside its navigation', (
    tester,
  ) async {
    await _pumpHome(
      tester,
      loadSummary: (queries) async => const ComplexQueryResponse(
        result: [
          ComplexQueryGroup(
            domain: LibraryHomeDomain.albums,
            title: 'Newest Albums',
            entries: [
              ComplexQueryEntry(
                id: 4,
                name: 'Album',
                queries: [],
                collectionType: CollectionType.album,
                coverArtMap: {},
                readonly: false,
              ),
            ],
          ),
        ],
      ),
    );

    expect(find.text('Newest Albums'), findsOneWidget);
    expect(find.text('Album'), findsOneWidget);
    expect(find.text('Tracks'), findsOneWidget);
  });
}
