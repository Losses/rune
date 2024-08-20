import 'dart:collection';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:player/widgets/flip_animation.dart';

const navigationBarHeight = 64.0;

class NavigationBarPlaceholder extends StatelessWidget {
  const NavigationBarPlaceholder({super.key});

  @override
  Widget build(BuildContext context) {
    return const SizedBox(height: navigationBarHeight);
  }
}

class NavigationItem {
  final String title;
  final String path;
  final bool hidden;
  final bool tappable;
  final List<NavigationItem>? children;

  NavigationItem(this.title, this.path,
      {this.hidden = false, this.tappable = true, this.children = const []});

  @override
  String toString() {
    return 'NavigationItem(title: $title, path: $path, hidden: $hidden)';
  }
}

class NavigationQuery {
  final HashMap<String, NavigationItem> _pathToItem = HashMap();
  final HashMap<String, String> _pathToParent = HashMap();
  final HashMap<String, List<NavigationItem>> _pathToChildren = HashMap();
  final List<NavigationItem> _rootItems = [];

  NavigationQuery(List<NavigationItem> items) {
    for (var item in items) {
      _addItem(item, null);
    }
  }

  void _addItem(NavigationItem item, String? parentPath) {
    if (item.hidden) return;

    _pathToItem[item.path] = item;
    if (parentPath != null) {
      _pathToParent[item.path] = parentPath;
      if (!_pathToChildren.containsKey(parentPath)) {
        _pathToChildren[parentPath] = [];
      }
      _pathToChildren[parentPath]!.add(item);
    } else {
      _rootItems.add(item);
    }

    final children = item.children;
    if (children != null) {
      for (var child in children) {
        _addItem(child, item.path);
      }
    }
  }

  NavigationItem? getItem(String? path) {
    return _pathToItem[path];
  }

  NavigationItem? getParent(String? path) {
    var parentPath = _pathToParent[path];
    if (parentPath != null) {
      return _pathToItem[parentPath];
    }
    return null;
  }

  List<NavigationItem>? getChildren(String? path) {
    return _pathToChildren[path];
  }

  List<NavigationItem>? getSiblings(String? path) {
    var parentPath = _pathToParent[path];
    if (parentPath == null) {
      // If there's no parent, it means this item is a root item
      // Its siblings are all other root items
      return _rootItems.toList();
    }

    var siblings = _pathToChildren[parentPath];
    if (siblings == null) {
      return null;
    }

    // Filter out the item itself from the list of siblings
    return siblings.toList();
  }
}

class NavigationBar extends StatefulWidget {
  final List<NavigationItem> items;
  final String defaultPath;
  late final NavigationQuery query;

  NavigationBar({super.key, required this.items, this.defaultPath = "/"}) {
    query = NavigationQuery(items);
  }

  @override
  NavigationBarState createState() => NavigationBarState();
}

class NavigationBarState extends State<NavigationBar> {
  String? _previousPath;
  bool playing = false;
  bool initialized = false;
  String fromKey = '';
  String toKey = '';

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
      final previousItem = widget.query.getItem(_previousPath);
      final currentItem = widget.query.getItem(path);

      if (previousItem != null && currentItem != null) {
        if (widget.query.getParent(path)?.path == _previousPath) {
          // parent to child
          playFlipAnimation(context, 'child:$_previousPath', 'title:$path');
        } else if (widget.query.getParent(_previousPath)?.path == path) {
          // child to parent
          playFlipAnimation(context, 'title:$_previousPath', 'child:$path');
        } else {}
      }
    }

    _previousPath = path;
  }

  void _onRouteSelected(NavigationItem route) {
    GoRouter.of(context).push(route.path);
  }

  void _onHeaderTap(BuildContext context) {
    final path = GoRouterState.of(context).fullPath;

    if (widget.query.getItem(path)?.tappable == false) {
      return;
    }

    setState(() {
      final parent = widget.query.getParent(path ?? widget.defaultPath);

      if (parent != null) {
        context.go(parent.path);
      }
    });
  }

  playFlipAnimation(BuildContext context, String from, String to) async {
    final flipAnimation = FlipAnimationManager.of(context);

    playing = true;
    fromKey = from;
    toKey = to;
    await flipAnimation?.flipAnimation(from, to);
    playing = false;
    setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    final path = GoRouterState.of(context).fullPath;
    final item = widget.query.getItem(path);
    final parent = widget.query.getParent(path);
    final slibings =
        widget.query.getSiblings(path)?.where((x) => !x.hidden).toList();

    final titleFlipKey = 'title:${item?.path}';

    final parentWidget = Row(
      mainAxisAlignment: MainAxisAlignment.start,
      children: parent != null
          ? [
              Padding(
                padding: const EdgeInsets.only(right: 12),
                child: GestureDetector(
                    onTap: () async {
                      if (playing) return;
                      _onHeaderTap(context);
                    },
                    child: SizedBox(
                        height: 80,
                        width: 320,
                        child: FlipText(
                          key: UniqueKey(),
                          flipKey: titleFlipKey,
                          text: parent.title,
                          hidden: playing,
                          scale: 6,
                          alpha: 80,
                        ))),
              )
            ]
          : [],
    );

    final childrenWidget = Row(
      mainAxisAlignment: MainAxisAlignment.start,
      children: slibings?.map((route) {
            final flipKey = 'child:${route.path}';

            return Padding(
              padding: const EdgeInsets.only(right: 24),
              child: GestureDetector(
                  onTap: () async {
                    if (playing) return;
                    _onRouteSelected(route);
                  },
                  child: FlipText(
                    key: UniqueKey(),
                    flipKey: flipKey,
                    text: route.title,
                    scale: 1.2,
                    alpha: route == item ? 255 : 100,
                    hidden: playing && (fromKey == flipKey || toKey == flipKey),
                  )),
            );
          }).toList() ??
          [],
    );

    return BackButtonListener(
        onBackButtonPressed: () async {
          final router = GoRouter.of(context);
          final canPop = router.canPop();

          if (!canPop) {
            if (parent != null) {
              router.go(parent.path);
            }
          }
          return !canPop;
        },
        child: Transform.translate(
          offset: const Offset(0, -40),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              parentWidget,
              Container(
                padding:
                    const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
                child: childrenWidget,
              )
            ],
          ),
        ));
  }
}
