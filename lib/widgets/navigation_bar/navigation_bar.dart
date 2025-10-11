import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/router/navigation.dart';
import '../../utils/navigation/navigation_item.dart';
import '../../utils/navigation/utils/escape_from_search.dart';
import '../../config/navigation_query.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/navigation_bar/navigation_back_button.dart';
import '../../providers/router_path.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/linux_custom_window_controls.dart';

import '../ax_reveal/ax_reveal.dart';
import '../rune_clickable.dart';

import 'parent_link.dart';
import 'slibing_link.dart';
import 'flip_animation_manager.dart';

const List<NavigationItem> emptySlibings = [];

class NavigationBar extends StatefulWidget {
  final String? path;

  const NavigationBar({super.key, required this.path});

  @override
  NavigationBarState createState() => NavigationBarState();
}

class NavigationBarState extends State<NavigationBar> {
  @override
  void didUpdateWidget(covariant NavigationBar oldWidget) {
    super.didUpdateWidget(oldWidget);

    if (oldWidget.path != widget.path) {
      _onRouteChanged(oldWidget.path, widget.path);
    }
  }

  void _onRouteChanged(String? previousPath, String? currentPath) {
    if (previousPath != null && currentPath != previousPath) {
      final previousItem = navigationQuery.getItem(previousPath, false);
      final currentItem = navigationQuery.getItem(currentPath, false);

      if (previousItem != null && currentItem != null) {
        if (navigationQuery.getParent(currentPath, false)?.path ==
            previousPath) {
          // parent to child
          playFlipAnimation(
            context,
            'child:$previousPath',
            'title:$previousPath',
          );
        } else if (navigationQuery.getParent(previousPath, false)?.path ==
            currentPath) {
          // child to parent
          playFlipAnimation(
            context,
            'title:$currentPath',
            'child:$currentPath',
          );
        } else {}
      }
    }
  }

  void _onRouteSelected(NavigationItem route) {
    final currentPath = $router.path;
    if (route.path == currentPath) return;

    $replace(route.path);
  }

  void _onHeaderTap(BuildContext context, NavigationItem? item) {
    final onTap = item?.onTap;

    if (onTap != null) {
      onTap(context);
      return;
    }

    setState(() {
      if (item != null) {
        $replace(item.path);
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
        final path = Provider.of<RouterPathProvider>(context).path;

        final item = navigationQuery.getItem(path, isZune);
        final parent = navigationQuery.getParent(path, isZune);
        final slibings = navigationQuery
            .getSiblings(path, isZune)
            ?.where((x) => !x.hidden)
            .toList();

        final titleFlipKey = 'title:${parent?.path}';

        final viewPadding = MediaQuery.of(context).viewPadding;

        late Widget parentWidget = Align(
          alignment: AlignmentGeometry.topLeft,
          child: SmallerOrEqualTo(
            deviceType: DeviceType.fish,
            builder: (context, isFish) {
              if (isFish) {
                return NavigationBackParent(viewPadding: viewPadding);
              }

              return parent != null
                  ? Padding(
                      padding: const EdgeInsets.only(right: 12),
                      child: SizedBox(
                        height: 92,
                        child: ParentLink(
                          titleFlipKey: titleFlipKey,
                          text: parent.titleBuilder(context),
                          onPressed: () => _onHeaderTap(context, parent),
                        ),
                      ),
                    )
                  : Container();
            },
          ),
        );

        final baseSlibings = (slibings ?? emptySlibings);
        final validSlibings = isZune
            ? baseSlibings
            : baseSlibings.where((x) => !x.zuneOnly).toList();

        final int currentItemIndex = validSlibings.indexWhere(
          (route) => route == item,
        );

        final childrenWidget = SmallerOrEqualTo(
          deviceType: DeviceType.fish,
          builder: (context, isFish) {
            return SmoothHorizontalScroll(
              builder: (context, scrollController) => SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                controller: scrollController,
                padding: isFish
                    ? const EdgeInsets.symmetric(horizontal: 20, vertical: 0)
                    : const EdgeInsets.only(
                        left: 20,
                        right: 20,
                        top: 0,
                        bottom: 12,
                      ),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.start,
                  children: validSlibings.toList().asMap().entries.map((entry) {
                    final route = entry.value;

                    final index = entry.key;
                    final isCurrent = currentItemIndex == index;

                    final int delay = !isCurrent
                        ? ((index - currentItemIndex).abs() * 100)
                        : 0;

                    return SlibingLink(
                      key: ValueKey('${parent?.path}/${route.path}'),
                      route: route,
                      isSelected: route == item,
                      delay: delay,
                      onPressed: () => _onRouteSelected(route),
                    );
                  }).toList(),
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
                offset: Offset(0 + viewPadding.left, -44 + viewPadding.top),
                child: Padding(
                  padding: Platform.isMacOS
                      ? const EdgeInsets.only(left: 24)
                      : const EdgeInsets.only(),
                  child: FocusTraversalGroup(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      mainAxisSize: MainAxisSize.max,
                      children: [parentWidget, childrenWidget],
                    ),
                  ),
                ),
              ),
            if (!isZune && !Platform.isWindows && !(Platform.isLinux &&
                context.watch<LinuxCustomWindowControlsProvider>().enabled))
              Positioned(
                top: 16 + viewPadding.top,
                right: 16 + viewPadding.right,
                child: AxReveal0(
                  child: RuneClickable(
                    child: Icon(
                      isSearch ? Symbols.close : Symbols.search,
                      size: 24,
                    ),
                    onPressed: () {
                      if (isSearch) {
                        escapeFromSearch();
                      } else {
                        $push('/search');
                      }
                    },
                  ),
                ),
              ),
          ],
        );
      },
    );
  }
}

class NavigationBackParent extends StatelessWidget {
  const NavigationBackParent({super.key, required this.viewPadding});

  final EdgeInsets viewPadding;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: EdgeInsetsDirectional.only(top: 20 + viewPadding.top),
      child: Align(
        alignment: Alignment.centerLeft,
        child: SizedBox(width: 60, height: 60, child: NavigationBackButton()),
      ),
    );
  }
}
