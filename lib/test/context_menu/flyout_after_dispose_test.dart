import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/utils/context_menu/collection_item_context_menu.dart';
import 'package:rune/utils/router/router_aware_flyout_controller.dart';
import 'package:rune/widgets/context_menu_wrapper.dart';

/// An item that owns its flyout controller the way the list items do.
class _Item extends StatefulWidget {
  final Future<List<Mix>> Function() loadMixes;

  /// A context that outlives the item, like the one of the page around a list.
  final BuildContext pageContext;

  const _Item(this.loadMixes, this.pageContext);

  @override
  State<_Item> createState() => _ItemState();
}

class _ItemState extends State<_Item> {
  final _controller = RouterAwareFlyoutController();
  final _attachKey = GlobalKey();

  @override
  void dispose() {
    super.dispose();
    _controller.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ContextMenuWrapper(
      contextAttachKey: _attachKey,
      contextController: _controller,
      onContextMenu: (position) => openCollectionItemContextMenu(
        position,
        widget.pageContext,
        _attachKey,
        _controller,
        CollectionType.album,
        1,
        'Album',
        null,
        null,
        widget.loadMixes,
      ),
      onMiddleClick: (_) {},
      child: const SizedBox(
        key: Key('item'),
        width: 120,
        height: 40,
        child: ColoredBox(color: Color(0xFF000000)),
      ),
    );
  }
}

class _Host extends StatefulWidget {
  final Future<List<Mix>> Function() loadMixes;

  const _Host(this.loadMixes, {super.key});

  @override
  State<_Host> createState() => _HostState();
}

class _HostState extends State<_Host> {
  bool large = true;

  void switchLayout() => setState(() => large = false);

  @override
  Widget build(BuildContext context) {
    // The list swaps its item widgets between layouts, which disposes the old state.
    return Center(
      child: large
          ? _Item(widget.loadMixes, context)
          : const SizedBox(key: Key('small-item'), width: 60, height: 20),
    );
  }
}

void main() {
  testWidgets('a menu whose item is gone when its data arrives shows nothing', (
    tester,
  ) async {
    // A desktop window, so the menu loads its data before it opens.
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1920, 1080);
    addTearDown(tester.view.reset);
    final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
    final load = Completer<List<Mix>>();
    final hostKey = GlobalKey<_HostState>();
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
          ChangeNotifierProvider<ResponsiveProvider>(
            create: (_) => ResponsiveProvider(sizes, () async => null),
          ),
        ],
        child: FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: _Host(() => load.future, key: hostKey),
        ),
      ),
    );

    final gesture = await tester.startGesture(
      tester.getCenter(find.byKey(const Key('item'))),
      kind: PointerDeviceKind.mouse,
      buttons: kSecondaryMouseButton,
    );
    await gesture.up();
    await tester.pump();

    // The menu is loading. The window shrinks and the list swaps the item.
    hostKey.currentState!.switchLayout();
    await tester.pump();
    expect(find.byKey(const Key('item')), findsNothing);

    load.complete(const []);
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));

    expect(tester.takeException(), isNull);
    expect(find.byType(MenuFlyout), findsNothing);
  });
}
