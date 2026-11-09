import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/library_open_state.dart';
import '../../utils/context_menu/collection_item_context_menu.dart';
import '../../utils/execute_middle_click_action.dart';

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
