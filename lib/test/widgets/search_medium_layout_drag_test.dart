import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/search/widgets/medium_screen_search_track_list.dart';
import 'package:rune/screens/search/widgets/search_card.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'package:rune/widgets/navigation_bar/page_content_frame.dart';
import 'package:rune/widgets/start_screen/providers/start_screen_layout_manager.dart';

class _Responsive extends ChangeNotifier implements ResponsiveProvider {
  @override
  DeviceType getActiveDeviceType(List<DeviceType> deviceTypes) => DeviceType.tv;
  @override
  bool smallerOrEqualTo(DeviceType type, [bool autoOrientation = true]) =>
      false;
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Screen extends ChangeNotifier implements ScreenSizeProvider {
  @override
  Size get screenSize => const Size(375, 812);
  @override
  Size get layoutSize => const Size(375, 812);
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Card extends SearchCard {
  const _Card({required super.index});

  @override
  State<_Card> createState() => _CardState();
}

class _CardState extends SearchCardState<_Card> {
  @override
  int getItemId() => widget.index;
  @override
  String getItemTitle() => 'Item ${widget.index}';
  @override
  Widget buildLeadingWidget(double size) => SizedBox.square(dimension: size);
  @override
  void onPressed(BuildContext context) {}
  @override
  void onMiddleClick(BuildContext context, Offset position) {}
  @override
  void onContextMenu(BuildContext context, Offset position) {}
}

void main() {
  testWidgets('a drag that starts on a card scrolls the medium search layout', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(375, 812);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    final controller = ScrollController();
    addTearDown(controller.dispose);
    final layout = StartScreenLayoutManager();
    addTearDown(layout.dispose);

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<ResponsiveProvider>.value(
            value: _Responsive(),
          ),
          ChangeNotifierProvider<ScreenSizeProvider>.value(value: _Screen()),
          ChangeNotifierProvider<StartScreenLayoutManager>.value(value: layout),
        ],
        child: FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          builder: (context, child) => RevealEffectContext(child: child!),
          // The structure the search page builds for the tablet layout.
          home: PageContentFrame(
            top: false,
            builder: (context) => SingleChildScrollView(
              controller: controller,
              padding: getScrollContainerPadding(context),
              child: MediumScreenSearchTrackList(
                items: {
                  CollectionType.album: [
                    for (var i = 0; i < 6; i++) _Card(index: i),
                  ],
                  CollectionType.track: [
                    for (var i = 6; i < 26; i++) _Card(index: i),
                  ],
                },
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    layout.playAnimations();
    await tester.pumpAndSettle();

    await tester.drag(find.text('Item 2'), const Offset(0, -300));
    await tester.pumpAndSettle();

    expect(controller.offset, greaterThan(200));
  });
}
