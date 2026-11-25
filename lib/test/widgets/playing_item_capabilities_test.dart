import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/playing_item_capabilities.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/status.dart';
import '../../screens/lyrics/lyrics.dart';
import '../../screens/lyrics/widgets/lyric_display.dart';
import '../../utils/navigation/controller_action.dart';
import '../../utils/navigation/controller_intent.dart';
import '../../utils/playing_item.dart';
import '../../widgets/playback_controller/constants/controller_items.dart';

const _hiddenRef = PlayingItemRefLibraryTrack(
  libraryId: 'service-record',
  track: 't1',
);
final _hiddenItem = PlayingItem.fromRequest(_hiddenRef);
const _localItem = PlayingItem.libraryItem(
  libraryId: 'local-library',
  fileUuid: '00000000-0000-0000-0000-000000000001',
);

/// What Rust declares for a track whose library has no lyrics for it: likes as
/// usual.
const _hiddenCapabilities = PlayingItemCapabilities(
  item: _hiddenRef,
  setLikedRequest: RequestCapabilityAvailable(),
  getLikedRequest: RequestCapabilityAvailable(),
  getLyricByTrackIdRequest: RequestCapabilityHidden(),
  getPrimaryColorByTrackIdRequest: RequestCapabilityAvailable(),
);

const _fullRef = PlayingItemRefLibraryTrack(
  libraryId: 'full-record',
  track: 's1',
);
final _fullItem = PlayingItem.fromRequest(_fullRef);

PlayingItemCapabilities _declared(
  PlayingItemRef item,
  RequestCapability lyrics,
) => PlayingItemCapabilities(
  item: item,
  setLikedRequest: const RequestCapabilityAvailable(),
  getLikedRequest: const RequestCapabilityAvailable(),
  getLyricByTrackIdRequest: lyrics,
  getPrimaryColorByTrackIdRequest: const RequestCapabilityAvailable(),
);

/// What Rust declares for a track whose library answers everything.
final _fullCapabilities = _declared(
  _fullRef,
  const RequestCapabilityAvailable(),
);

ControllerEntry get _lyricEntry =>
    controllerItems.firstWhere((entry) => entry.id == 'lyric');

class _StatusProvider extends ChangeNotifier implements PlaybackStatusProvider {
  _StatusProvider(this._item);

  PlayingItem? _item;

  @override
  final PlaybackStatusState playbackStatus = PlaybackStatusState();

  @override
  PlayingItem? get playingItem => _item;

  set playingItem(PlayingItem? item) {
    _item = item;
    notifyListeners();
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _RequestCapabilities extends ChangeNotifier
    implements RequestCapabilitiesProvider {
  RequestCapability next = const RequestCapabilityAvailable();

  @override
  RequestCapability get nextRequest => next;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  test('a declaration counts only for the item it names', () {
    final provider = PlayingItemCapabilitiesProvider(
      changes: const Stream.empty(),
      initial: _hiddenCapabilities,
    );
    addTearDown(provider.dispose);
    RequestCapability lyrics(PlayingItem? item) =>
        provider.capabilityFor(item, (c) => c.getLyricByTrackIdRequest);

    expect(lyrics(_hiddenItem), const RequestCapabilityHidden());
    expect(lyrics(_localItem), const RequestCapabilityAvailable());
    expect(lyrics(null), const RequestCapabilityAvailable());

    final none = PlayingItemCapabilitiesProvider(changes: const Stream.empty());
    addTearDown(none.dispose);
    expect(
      none.capabilityFor(_hiddenItem, (c) => c.getLyricByTrackIdRequest),
      const RequestCapabilityAvailable(),
    );
  });

  test(
    'a declaration for the next item leaves the shown one as it was',
    () async {
      final declarations = StreamController<PlayingItemCapabilities>();
      final provider = PlayingItemCapabilitiesProvider(
        changes: declarations.stream,
        initial: _hiddenCapabilities,
      );
      addTearDown(() {
        provider.dispose();
        unawaited(declarations.close());
      });
      RequestCapability lyrics(PlayingItem item) =>
          provider.capabilityFor(item, (c) => c.getLyricByTrackIdRequest);

      // Rust declares the next item before the status that names it arrives.
      declarations.add(_fullCapabilities);
      await Future<void>.delayed(Duration.zero);
      expect(lyrics(_hiddenItem), const RequestCapabilityHidden());
      expect(lyrics(_fullItem), const RequestCapabilityAvailable());

      // Only the last few items are kept; the oldest goes first.
      for (final track in ['s2', 's3', 's4']) {
        declarations.add(
          _declared(
            PlayingItemRefLibraryTrack(
              libraryId: 'full-record',
              track: track,
            ),
            const RequestCapabilityHidden(),
          ),
        );
      }
      await Future<void>.delayed(Duration.zero);
      expect(lyrics(_hiddenItem), const RequestCapabilityAvailable());
    },
  );

  testWidgets('a shortcut does nothing a hidden or disabled button would not', (
    tester,
  ) async {
    final library = _RequestCapabilities();
    late BuildContext context;
    await tester.pumpWidget(
      ChangeNotifierProvider<RequestCapabilitiesProvider>.value(
        value: library,
        child: Builder(
          builder: (built) {
            context = built;
            return const SizedBox.shrink();
          },
        ),
      ),
    );
    final next = controllerItems.firstWhere((entry) => entry.id == 'next');
    bool shortcut() =>
        ControllerAction(context).isEnabled(ControllerIntent(next));

    expect(shortcut(), isTrue);
    library.next = const RequestCapabilityHidden();
    expect(shortcut(), isFalse);
    library.next = const RequestCapabilityDisabled(
      reason: CapabilityReason.ownerOnly,
    );
    expect(shortcut(), isFalse);
  });

  testWidgets('the lyrics entry and its shortcut follow the playing item', (
    tester,
  ) async {
    final declarations = StreamController<PlayingItemCapabilities>();
    addTearDown(declarations.close);
    final capabilities = PlayingItemCapabilitiesProvider(
      changes: declarations.stream,
    );
    addTearDown(capabilities.dispose);
    final status = _StatusProvider(_hiddenItem);
    late BuildContext context;
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<PlaybackStatusProvider>.value(value: status),
          ChangeNotifierProvider<PlayingItemCapabilitiesProvider>.value(
            value: capabilities,
          ),
        ],
        child: Builder(
          builder: (built) {
            context = built;
            return const SizedBox.shrink();
          },
        ),
      ),
    );
    RequestCapability offered() => _lyricEntry.capability(context, false);
    bool shortcut() =>
        ControllerAction(context).isEnabled(ControllerIntent(_lyricEntry));

    // Before Rust declares anything, lyrics are offered as they always were.
    expect(offered(), const RequestCapabilityAvailable());
    expect(shortcut(), isTrue);

    declarations.add(_hiddenCapabilities);
    await tester.pump();
    expect(offered(), const RequestCapabilityHidden());
    expect(shortcut(), isFalse);

    // The next track is a local one, which the declaration does not name.
    status.playingItem = _localItem;
    await tester.pump();
    expect(offered(), const RequestCapabilityAvailable());
    expect(shortcut(), isTrue);
  });

  group('the lyrics page', () {
    late StreamController<PlayingItemCapabilities> declarations;
    late _StatusProvider status;
    late List<PlayingItem?> asked;

    /// Shows the page for [first], whose lyrics [load] reads, after Rust
    /// declared [declared].
    Future<void> showPage(
      WidgetTester tester, {
      required PlayingItem first,
      required List<PlayingItemCapabilities> declared,
      required Future<List<LyricContentLine>> Function(PlayingItem? item) load,
    }) async {
      // A band-sized window, whose layout draws the lyrics alone.
      tester.view.physicalSize = const Size(800, 140);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      declarations = StreamController<PlayingItemCapabilities>();
      addTearDown(() {
        unawaited(declarations.close());
      });
      final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
      addTearDown(sizes.dispose);
      final capabilities = PlayingItemCapabilitiesProvider(
        changes: declarations.stream,
      );
      addTearDown(capabilities.dispose);
      for (final declaration in declared) {
        declarations.add(declaration);
      }
      status = _StatusProvider(first);
      asked = [];
      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
            ChangeNotifierProvider<ResponsiveProvider>(
              create: (_) => ResponsiveProvider(sizes, () async => null),
            ),
            ChangeNotifierProvider<PlaybackStatusProvider>.value(value: status),
            ChangeNotifierProvider<PlayingItemCapabilitiesProvider>.value(
              value: capabilities,
            ),
          ],
          child: FluentApp(
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            locale: const Locale('en'),
            home: LyricsPage(
              loadLyrics: (item) {
                asked.add(item);
                return load(item);
              },
            ),
          ),
        ),
      );
      // Lets the screen size provider save the window size it read.
      await tester.pump(const Duration(seconds: 2));
    }

    testWidgets('asks for no lyrics when its library offers none', (tester) async {
      await showPage(
        tester,
        first: _hiddenItem,
        declared: [_hiddenCapabilities],
        load: (_) async => const [],
      );

      expect(asked, isEmpty);
    });

    testWidgets('follows the songs it shows: one without lyrics, one with, back', (
      tester,
    ) async {
      await showPage(
        tester,
        first: _hiddenItem,
        declared: [_hiddenCapabilities, _fullCapabilities],
        load: (_) async => const [],
      );
      expect(asked, isEmpty);

      status.playingItem = _fullItem;
      await tester.pump(const Duration(seconds: 1));
      expect(asked, [_fullItem]);

      status.playingItem = _hiddenItem;
      await tester.pump(const Duration(seconds: 1));
      expect(asked, [_fullItem]);
    });

    testWidgets(
      'keeps what the shown song declared when the next is declared',
      (tester) async {
        await showPage(
          tester,
          first: _hiddenItem,
          declared: [_hiddenCapabilities],
          load: (_) async => const [],
        );

        // Rust declares the next song before the status that names it.
        declarations.add(_fullCapabilities);
        await tester.pump(const Duration(seconds: 1));
        expect(asked, isEmpty);
      },
    );

    testWidgets('asks once the library offers its lyrics', (tester) async {
      await showPage(
        tester,
        first: _fullItem,
        declared: [
          _declared(
            _fullRef,
            const RequestCapabilityDisabled(
              reason: CapabilityReason.accountRoleMissing,
            ),
          ),
        ],
        load: (_) async => const [],
      );
      // Disabled: it may offer them later, and nothing is asked until then.
      expect(asked, isEmpty);

      declarations.add(_fullCapabilities);
      await tester.pump(const Duration(seconds: 1));
      expect(asked, [_fullItem]);
    });

    testWidgets('shows the lyrics it asked for last, whichever answer comes '
        'first', (tester) async {
      final answers = <Completer<List<LyricContentLine>>>[];
      await showPage(
        tester,
        first: _fullItem,
        declared: [_fullCapabilities],
        load: (_) {
          final answer = Completer<List<LyricContentLine>>();
          answers.add(answer);
          return answer.future;
        },
      );
      // The library stops offering the song's lyrics for a while, as until its
      // service signs in again, and then offers them again.
      declarations.add(_declared(_fullRef, const RequestCapabilityHidden()));
      await tester.pump(const Duration(seconds: 1));
      declarations.add(_fullCapabilities);
      await tester.pump(const Duration(seconds: 1));
      expect(answers, hasLength(2));

      LyricContentLine line(String content) => LyricContentLine(
        startTime: 0,
        endTime: 600000,
        sections: [
          LyricContentLineSection(
            startTime: 0,
            endTime: 600000,
            content: content,
          ),
        ],
      );
      answers[1].complete([line('asked last')]);
      await tester.pump();
      answers[0].complete([line('asked first')]);
      await tester.pump();
      // The next progress draws the lyrics again.
      status.playbackStatus.progressSeconds = 1;
      status.playingItem = _fullItem;
      await tester.pump(const Duration(seconds: 1));

      final shown = tester.widget<LyricsDisplay>(find.byType(LyricsDisplay));
      expect(shown.lyrics.single.sections.single.content, 'asked last');
    });
  });
}
