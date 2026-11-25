import 'dart:async';
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../providers/library_open_state.dart';
import '../../providers/playing_item_capabilities.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/playing_item.dart';
import '../../widgets/playback_controller/like_button.dart';

void main() {
  // A rejected write reaches the global handler, which shows its localized
  // message.
  for (final kind in [
    RequestErrorKind.notFound,
    RequestErrorKind.invalidInput,
  ]) {
    testWidgets('a like refused as $kind is left to the global handler', (
      tester,
    ) async {
      final failure = RequestFailure('SetLikedRequest', kind, 'refused');
      await tester.pumpWidget(
        ChangeNotifierProvider<LibraryOpenStateProvider>(
          create: (_) => _LibraryOpenStateProvider(),
          child: FluentApp(
            home: LikeButton(
              item: const PlayingItem.libraryItem(
                libraryId: 'library-uuid',
                fileUuid: 'file-uuid',
              ),
              fetchLikedStatus: (_) async => false,
              setLikedStatus: (_, _) async => throw failure,
            ),
          ),
        ),
      );

      // The button starts the request without awaiting it, so a failure it
      // does not catch is an uncaught error of the zone that tapped it. In the
      // app that is the root zone, whose handler shows the failure.
      final escaped = <Object>[];
      await runZonedGuarded(() async {
        await tester.tap(find.byIcon(Symbols.favorite));
        await tester.pump();
      }, (error, _) => escaped.add(error));
      expect(escaped, [same(failure)]);
    });
  }

  const serviceRef = PlayingItemRefLibraryTrack(
    libraryId: 'service-record',
    track: 't1',
  );
  final serviceItem = PlayingItem.fromRequest(serviceRef);
  Widget likeButton({
    required Future<bool> Function(PlayingItem item) fetch,
    PlayingItemCapabilitiesProvider? capabilities,
  }) => MultiProvider(
    providers: [
      ChangeNotifierProvider<LibraryOpenStateProvider>(
        create: (_) => _LibraryOpenStateProvider(),
      ),
      if (capabilities != null)
        ChangeNotifierProvider<PlayingItemCapabilitiesProvider>.value(
          value: capabilities,
        ),
    ],
    child: FluentApp(
      home: LikeButton(item: serviceItem, fetchLikedStatus: fetch),
    ),
  );
  double? heartFill(WidgetTester tester) =>
      tester.widget<Icon>(find.byIcon(Symbols.favorite)).fill;

  testWidgets('a like asked of a library needing a sign-in stays unknown', (
    tester,
  ) async {
    const failure = RequestFailure(
      'GetLikedRequest',
      RequestErrorKind.permissionDenied,
      'needs a new sign-in',
      RequestFailureCauseServiceSignInRequired(recordId: 'service-record'),
    );
    final escaped = <Object>[];
    await runZonedGuarded(() async {
      await tester.pumpWidget(likeButton(fetch: (_) async => throw failure));
      await tester.pump();
    }, (error, _) => escaped.add(error));

    expect(escaped, isEmpty);
    expect(heartFill(tester), 0);
  });

  testWidgets('the like is asked for again once the library answers', (
    tester,
  ) async {
    final declarations = StreamController<PlayingItemCapabilities>();
    addTearDown(declarations.close);
    PlayingItemCapabilities declared(RequestCapability likes) =>
        PlayingItemCapabilities(
          item: serviceRef,
          setLikedRequest: likes,
          getLikedRequest: likes,
          getLyricByTrackIdRequest: const RequestCapabilityHidden(),
          getPrimaryColorByTrackIdRequest: const RequestCapabilityAvailable(),
        );
    final capabilities = PlayingItemCapabilitiesProvider(
      changes: declarations.stream,
      initial: declared(
        const RequestCapabilityDisabled(
          reason: CapabilityReason.accountRoleMissing,
        ),
      ),
    );
    addTearDown(capabilities.dispose);
    var asked = 0;
    await tester.pumpWidget(
      likeButton(
        capabilities: capabilities,
        fetch: (_) async {
          asked += 1;
          return true;
        },
      ),
    );
    await tester.pump();

    // Until the library answers there is nothing to ask and nothing to like.
    expect(asked, 0);
    expect(find.byIcon(Symbols.favorite), findsNothing);

    declarations.add(declared(const RequestCapabilityAvailable()));
    await tester.pumpAndSettle();
    expect(asked, 1);
    expect(heartFill(tester), 1);
  });

  testWidgets('a like set elsewhere, as from a menu, shows on its item only', (
    tester,
  ) async {
    // The signal arrives in a microtask, and the heart is drawn on the next
    // frame.
    Future<void> likeSet(PlayingItemRef item, bool liked) async {
      assignRustSignal['SetLikedResponse']!(
        SetLikedResponse(item: item, liked: liked).bincodeSerialize(),
        Uint8List(0),
      );
      await tester.pump();
      await tester.pump();
    }

    await tester.pumpWidget(likeButton(fetch: (_) async => false));
    await tester.pump();
    expect(heartFill(tester), 0);

    await likeSet(
      const PlayingItemRefLibraryTrack(
        libraryId: 'other-record',
        track: 'i.other',
      ),
      true,
    );
    expect(heartFill(tester), 0);

    await likeSet(serviceRef, true);
    expect(heartFill(tester), 1);

    await likeSet(serviceRef, false);
    expect(heartFill(tester), 0);
  });

  testWidgets('the next track declared first leaves this one waiting', (
    tester,
  ) async {
    final declarations = StreamController<PlayingItemCapabilities>();
    addTearDown(declarations.close);
    final capabilities = PlayingItemCapabilitiesProvider(
      changes: declarations.stream,
      initial: PlayingItemCapabilities(
        item: serviceRef,
        setLikedRequest: const RequestCapabilityDisabled(
          reason: CapabilityReason.accountRoleMissing,
        ),
        getLikedRequest: const RequestCapabilityDisabled(
          reason: CapabilityReason.accountRoleMissing,
        ),
        getLyricByTrackIdRequest: const RequestCapabilityHidden(),
        getPrimaryColorByTrackIdRequest: const RequestCapabilityAvailable(),
      ),
    );
    addTearDown(capabilities.dispose);
    var asked = 0;
    await tester.pumpWidget(
      likeButton(
        capabilities: capabilities,
        fetch: (_) async {
          asked += 1;
          return true;
        },
      ),
    );
    await tester.pump();

    // Rust declares the next track, which can be liked, before the status
    // that names it: this one still waits for its library.
    declarations.add(
      const PlayingItemCapabilities(
        item: PlayingItemRefLibraryTrack(
          libraryId: 'service-record',
          track: 't2',
        ),
        setLikedRequest: RequestCapabilityAvailable(),
        getLikedRequest: RequestCapabilityAvailable(),
        getLyricByTrackIdRequest: RequestCapabilityHidden(),
        getPrimaryColorByTrackIdRequest: RequestCapabilityAvailable(),
      ),
    );
    await tester.pumpAndSettle();
    expect(asked, 0);
    expect(find.byIcon(Symbols.favorite), findsNothing);
  });
}

class _LibraryOpenStateProvider extends ChangeNotifier
    implements LibraryOpenStateProvider {
  @override
  bool get isReadOnlyReplica => false;

  @override
  LibraryRecordSummary? get openRecord => null;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
