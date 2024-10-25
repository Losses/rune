import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rune/widgets/navigation_bar/navigation_back_button.dart';

import '../../utils/navigation/navigation_item.dart';
import '../../utils/navigation/utils/escape_from_search.dart';
import '../../config/navigation_query.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../providers/responsive_providers.dart';

import 'parent_link.dart';
import 'slibing_link.dart';
import 'flip_animation_manager.dart';

const List<NavigationItem> emptySlibings = [];

class NavigationBar extends StatefulWidget {
  final String defaultPath;

  const NavigationBar({super.key, this.defaultPath = "/"});

  @override
  NavigationBarState createState() => NavigationBarState();
}

class NavigationBarState extends State<NavigationBar> {
  String? _previousPath;
  bool initialized = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    if (!initialized) {
      final path = GoRouterState.of(context).fullPath;
      _previousPath = path;
      initialized = true;
    } else {
      _onRouteChanged();
    }
  }

  void _onRouteChanged() {
    final path = GoRouterState.of(context).fullPath;

    if (_previousPath != null && path != _previousPath) {
      final previousItem = navigationQuery.getItem(_previousPath, false);
      final currentItem = navigationQuery.getItem(path, false);

      if (previousItem != null && currentItem != null) {
        if (navigationQuery.getParent(path, false)?.path == _previousPath) {
          // parent to child
          playFlipAnimation(
              context, 'child:$_previousPath', 'title:$_previousPath');
        } else if (navigationQuery.getParent(_previousPath, false)?.path ==
            path) {
          // child to parent
          playFlipAnimation(context, 'title:$path', 'child:$path');
        } else {}
      }
    }

    _previousPath = path;
  }

  void _onRouteSelected(NavigationItem route) {
    if (route.path == _previousPath) return;

    GoRouter.of(context).replace(route.path);
  }

  void _onHeaderTap(BuildContext context, NavigationItem? item) {
    if (item?.tappable == false) return;

    setState(() {
      if (item != null) {
        context.go(item.path);
      }
    });
  }

  playFlipAnimation(BuildContext context, String from, String to) async {
    final flipAnimation = FlipAnimationManager.of(context);
    await flipAnimation?.flipAnimation(from, to);
  }

  @override
  Widget build(BuildContext context) {
    return SmallerOrEqualTo(
      deviceType: DeviceType.zune,
      builder: (context, isZune) {
        final path = GoRouterState.of(context).fullPath;
        final item = navigationQuery.getItem(path, isZune);
        final parent = navigationQuery.getParent(path, isZune);
        final slibings = navigationQuery
            .getSiblings(path, isZune)
            ?.where((x) => !x.hidden)
            .toList();

        final titleFlipKey = 'title:${parent?.path}';

        late Widget parentWidget = SmallerOrEqualTo(
            deviceType: DeviceType.fish,
            builder: (context, isFish) {
              if (isFish) {
                return const Padding(
                  padding: EdgeInsetsDirectional.only(top: 20),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: SizedBox(
                      width: 60,
                      height: 60,
                      child: NavigationBackButton(),
                    ),
                  ),
                );
              }

              return parent != null
                  ? Padding(
                      padding: const EdgeInsets.only(right: 12),
                      child: SizedBox(
                        height: 80,
                        width: 320,
                        child: ParentLink(
                          titleFlipKey: titleFlipKey,
                          text: parent.title,
                          onPressed: () => _onHeaderTap(context, parent),
                        ),
                      ),
                    )
                  : Container();
            });

        final baseSlibings = (slibings ?? emptySlibings);
        final validSlibings = isZune
            ? baseSlibings
            : baseSlibings.where((x) => !x.zuneOnly).toList();

        final int currentItemIndex =
            validSlibings.indexWhere((route) => route == item);

        final childrenWidget = SmallerOrEqualTo(
          deviceType: DeviceType.fish,
          builder: (context, isFish) {
            return SmoothHorizontalScroll(
              builder: (context, scrollController) => SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                controller: scrollController,
                padding: isFish
                    ? const EdgeInsets.symmetric(horizontal: 20, vertical: 0)
                    : const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.start,
                  children: validSlibings.toList().asMap().entries.map(
                    (entry) {
                      final route = entry.value;

                      final index = entry.key;
                      final isCurrent = currentItemIndex == index;

                      final int? delay = !isCurrent
                          ? ((index - currentItemIndex).abs() * 100)
                          : null;

                      return SlibingLink(
                        key: ValueKey('${parent?.path}/${route.path}'),
                        route: route,
                        isSelected: route == item,
                        delay: delay,
                        onTap: () => _onRouteSelected(route),
                      );
                    },
                  ).toList(),
                ),
              ),
            );
          },
        );

        final isSearch = path == '/search';

        return Stack(
          children: [
            if (isZune || !isSearch)
              Transform.translate(
                offset: const Offset(0, -40),
                child: FocusTraversalGroup(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    mainAxisSize: MainAxisSize.max,
                    children: [
                      parentWidget,
                      childrenWidget,
                    ],
                  ),
                ),
              ),
            if (!isZune)
              Positioned(
                top: 16,
                right: 16,
                child: IconButton(
                  icon: Icon(
                    isSearch ? Symbols.close : Symbols.search,
                    size: 24,
                  ),
                  onPressed: () => {
                    if (isSearch)
                      {escapeFromSearch(context)}
                    else
                      {context.push('/search')}
                  },
                ),
              ),
          ],
        );
      },
    );
  }
}
