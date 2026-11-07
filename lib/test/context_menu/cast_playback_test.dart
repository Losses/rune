import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/library_path.dart';
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

  testWidgets('collection Play on is available for the open library', (
    tester,
  ) async {
    await tester.pumpWidget(
      ChangeNotifierProvider<LibraryPathProvider>(
        create: (_) => _TestLibraryPathProvider('/music'),
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

    expect(find.text('true'), findsOneWidget);
  });
}

class _TestLibraryPathProvider extends LibraryPathProvider {
  _TestLibraryPathProvider(this.path) : super(null);

  final String path;

  @override
  String? get currentPath => path;
}
