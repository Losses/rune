import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/kept_clear.dart';
import '../../utils/router/navigation.dart';
import '../../utils/router/history.dart';
import '../../utils/router/route_entry.dart';
import '../../utils/navigation/header_parent.dart';
import '../../utils/navigation/navigation_item.dart';
import '../../utils/navigation/utils/escape_from_search.dart';
import '../../config/navigation_query.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/navigation_bar/navigation_back_button.dart';
import '../../providers/router_path.dart';
import '../../utils/native_window_metrics.dart';
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
  // The parent the header showed in the last build, the page of history it
  // named if it named one, and the page shown.
  String? _shownParentPath;
  RouteEntry? _shownOpener;
  RouteEntry? _shownPage;

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
        // A title that names a page of history names that page and no other
        // with its path: an artist opened over an album titled with another
        // artist is no return to that one, and closing it opens no album.
        final opener = headerOpener(currentPath, $history, false);
        if (headerParent(currentPath, $history, false)?.path == previousPath &&
            (opener == null || identical(opener, _shownPage))) {
          // parent to child
          playFlipAnimation(
            context,
            'child:$previousPath',
            'title:$previousPath',
          );
        } else if (_shownParentPath == currentPath &&
            (_shownOpener == null ||
                identical($history.currentPage, _shownOpener))) {
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

    // A link the header made for the page shown, such as one to another
    // section of an artist, opens as it says. A link to a page of the
    // navigation replaces this page with it.
    final onTap = route.onTap;
    if (onTap != null && navigationQuery.getItem(route.path, false) == null) {
      onTap(context);
      return;
    }

    $replace(route.path);
  }

  void _onHeaderTap(BuildContext context, NavigationItem? item) {
    final onTap = item?.onTap;

    if (onTap != null) {
      onTap(context);
      return;
    }

    if (item == null) return;

    // Up to the page this one was opened from is back to that page as it was
    // left, scrolled where it was. Another parent opens afresh.
    switch (headerTapAction(item.path, $router.path, $history)) {
      case HeaderTapAction.none:
        return;
      case HeaderTapAction.pop:
        $pop();
      case HeaderTapAction.replace:
        setState(() {
          $replace(item.path);
        });
    }
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

        final row = headerRow(path, $history, isZune);
        final item = row.current;
        final parent = headerParent(path, $history, isZune);
        _shownParentPath = parent?.path;
        _shownOpener = headerOpener(path, $history, isZune);
        _shownPage = $history.currentPage;
        final slibings = row.links?.where((x) => !x.hidden).toList();

        final titleFlipKey = 'title:${parent?.path}';

        final keptClear = context.keptClear;

        late Widget parentWidget = Align(
          alignment: AlignmentGeometry.topLeft,
          child: SmallerOrEqualTo(
            deviceType: DeviceType.fish,
            builder: (context, isFish) {
              if (isFish) {
                return const NavigationBackParent();
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
                      key: ValueKey(
                        '${parent?.path}/${route.linkKey ?? route.path}',
                      ),
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
              // The titles keep their place under the status bar, the links
              // keep clear of the sides.
              Transform.translate(
                offset: Offset(0, -44 + keptClear.top),
                child: Padding(
                  padding: EdgeInsets.only(
                    left:
                        keptClear.left +
                        (Platform.isMacOS
                            ? nativeWindowLogicalToLocal(context, 24)
                            : 0),
                    right: keptClear.right,
                  ),
                  child: FocusTraversalGroup(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      mainAxisSize: MainAxisSize.max,
                      children: [parentWidget, childrenWidget],
                    ),
                  ),
                ),
              ),
            if (!isZune &&
                !Platform.isWindows &&
                !(Platform.isLinux &&
                    context.watch<LinuxCustomWindowControlsProvider>().enabled))
              Positioned(
                top: 16 + keptClear.top,
                right: 16 + keptClear.right,
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
  const NavigationBackParent({super.key});

  @override
  Widget build(BuildContext context) {
    // The bar around it is already below the status bar.
    return Padding(
      padding: const EdgeInsetsDirectional.only(top: 20),
      child: Align(
        alignment: Alignment.centerLeft,
        child: SizedBox(width: 60, height: 60, child: NavigationBackButton()),
      ),
    );
  }
}
