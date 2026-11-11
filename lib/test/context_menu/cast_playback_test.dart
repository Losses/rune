import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/library_open_state.dart';
import '../../utils/context_menu/collection_item_context_menu.dart';
import '../../utils/execute_middle_click_action.dart';
import '../../utils/api/operate_playback_with_mix_query.dart';

void main() {
  test(
    'track cast follows its setting and collection cast always replaces',
    () {
      expect(castTrackBehavior(true), (
        operateMode: PlaylistOperateMode.playNext,
        instantlyPlay: false,
      ));
      expect(castTrackBehavior(false), (
        operateMode: PlaylistOperateMode.replace,
        instantlyPlay: true,
      ));
      expect(castTrackBehaviorForSetting('replace'), castTrackBehavior(false));
      expect(castTrackBehaviorForSetting('playNext'), castTrackBehavior(true));
      expect(castTrackBehaviorForSetting(null), castTrackBehavior(false));
      expect(castCollectionBehavior(), (
        operateMode: PlaylistOperateMode.replace,
        instantlyPlay: true,
      ));
      expect(
        castTargetFor('renderer-fingerprint', true),
        const CastTarget(fingerprint: 'renderer-fingerprint', attach: true),
      );
    },
  );

  test(
    'a failed attach cast restores the prior player and play state',
    () async {
      String? attached;
      var detached = false;
      var resumed = false;
      await restoreCastAttachmentAfterLoadFailure(
        previousRenderer: 'renderer-a',
        wasPlaying: true,
        attach: (fingerprint) async => attached = fingerprint,
        detach: () async => detached = true,
        resume: () => resumed = true,
      );
      expect(attached, 'renderer-a');
      expect(detached, isFalse);
      expect(resumed, isTrue);
    },
  );

  test('a failed first attach detaches and resumes local playback', () async {
    var detached = false;
    var resumed = false;
    await restoreCastAttachmentAfterLoadFailure(
      previousRenderer: null,
      wasPlaying: true,
      attach: (_) async {},
      detach: () async => detached = true,
      resume: () => resumed = true,
    );
    expect(detached, isTrue);
    expect(resumed, isTrue);
  });

  testWidgets('collection Play on is hidden for a remote-control library', (
    tester,
  ) async {
    await tester.pumpWidget(
      ChangeNotifierProvider<LibraryOpenStateProvider>(
        create: (_) => _TestLibraryOpenStateProvider(remote: true),
        child: FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: Builder(
            builder: (context) {
              final menu = buildLargeScreenCollectionItemContextMenu(
                context,
                CollectionType.album,
                1,
                'Album',
                const [],
              );
              return Text(
                menu.items
                    .whereType<MenuFlyoutItem>()
                    .any(
                      (item) =>
                          item.text is Text &&
                          (item.text as Text).data == 'Play on',
                    )
                    .toString(),
              );
            },
          ),
        ),
      ),
    );

    expect(find.text('false'), findsOneWidget);
  });
}

class _TestLibraryOpenStateProvider extends ChangeNotifier
    implements LibraryOpenStateProvider {
  _TestLibraryOpenStateProvider({required this.remote});

  final bool remote;

  @override
  bool get isRemoteControl => remote;

  @override
  String? get catalogLibraryId => null;

  @override
  bool get isReadOnlyReplica => false;

  @override
  LibraryRecordSummary? get openRecord => const LibraryRecordSummary(
    recordId: 'remote-record',
    alias: 'Renderer',
    kind: LibraryRecordKindSummaryRemoteControl(fingerprint: 'renderer'),
  );

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
