import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../utils/context_menu/collection_item_context_menu.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../widgets/context_menu_wrapper.dart';
import 'context_menu_test_harness.dart';

const _collectionKey = Key('collection');

class _CollectionTile extends StatefulWidget {
  const _CollectionTile({required this.loadMixes});

  final Future<List<Mix>> Function() loadMixes;

  @override
  State<_CollectionTile> createState() => _CollectionTileState();
}

class _CollectionTileState extends State<_CollectionTile> {
  final _controller = RouterAwareFlyoutController();
  final _attachKey = GlobalKey();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ContextMenuWrapper(
      contextAttachKey: _attachKey,
      contextController: _controller,
      onContextMenu: (position) => openCollectionItemContextMenu(
        position,
        context,
        _attachKey,
        _controller,
        CollectionType.playlist,
        7,
        'Road Trip',
        null,
        false,
        widget.loadMixes,
      ),
      onMiddleClick: (_) {},
      child: const SizedBox(
        key: _collectionKey,
        width: 120,
        height: 40,
        child: ColoredBox(color: Color(0xFF000000)),
      ),
    );
  }
}

void main() {
  testWidgets('a collection of a library without mixes opens its menu', (
    tester,
  ) async {
    var mixLoads = 0;
    await openContextMenu(
      tester,
      record: subsonicRecord,
      capabilities: NoMixCapabilities(),
      target: _CollectionTile(
        loadMixes: () async {
          mixLoads += 1;
          throw mixesUnsupported;
        },
      ),
      targetKey: _collectionKey,
    );

    expect(find.byType(ContentDialog), findsNothing);
    expect(find.byType(MenuFlyout), findsOneWidget);
    expect(find.text('Start Playing'), findsOneWidget);
    expect(find.text('Add to Queue'), findsOneWidget);
    expect(find.text('Add to Mix'), findsNothing);
    expect(mixLoads, 0);
  });

  testWidgets('a collection of a library that keeps mixes offers them', (
    tester,
  ) async {
    var mixLoads = 0;
    await openContextMenu(
      tester,
      record: folderRecord,
      capabilities: null,
      target: _CollectionTile(
        loadMixes: () async {
          mixLoads += 1;
          return const [
            Mix(id: 5, name: 'Evening', group: '', locked: false, mode: null),
          ];
        },
      ),
      targetKey: _collectionKey,
    );

    expect(find.byType(MenuFlyout), findsOneWidget);
    expect(find.text('Add to Mix'), findsOneWidget);
    expect(mixLoads, 1);

    await tester.tap(find.text('Add to Mix'));
    await tester.pumpAndSettle();
    expect(find.text('Evening'), findsOneWidget);
  });
}
