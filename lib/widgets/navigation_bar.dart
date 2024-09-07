import 'dart:async';
import 'dart:collection';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/flip_animation.dart';

const navigationBarHeight = 64.0 + 40;

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
      final previousItem = widget.query.getItem(_previousPath);
      final currentItem = widget.query.getItem(path);

      if (previousItem != null && currentItem != null) {
        if (widget.query.getParent(path)?.path == _previousPath) {
          // parent to child
          playFlipAnimation(
              context, 'child:$_previousPath', 'title:$_previousPath');
        } else if (widget.query.getParent(_previousPath)?.path == path) {
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

  List<Timer> _slibingAnimationFutures = [];
  List<double> _slibingOpacities = [];
  NavigationItem? _lastParent;

  @override
  void dispose() {
    super.dispose();

    _disposeAnimations();
  }

  _disposeAnimations() {
    for (final x in _slibingAnimationFutures) {
      x.cancel();
    }
  }

  _resetAnimations() {
    _slibingAnimationFutures = [];
    _slibingOpacities = [];
  }

  @override
  Widget build(BuildContext context) {
    final path = GoRouterState.of(context).fullPath;
    final item = widget.query.getItem(path);
    final parent = widget.query.getParent(path);
    final slibings =
        widget.query.getSiblings(path)?.where((x) => !x.hidden).toList();

    final titleFlipKey = 'title:${parent?.path}';

    final parentWidget = Row(
      mainAxisAlignment: MainAxisAlignment.start,
      children: parent != null
          ? [
              Padding(
                padding: const EdgeInsets.only(right: 12),
                child: GestureDetector(
                    onTap: () async {
                      _onHeaderTap(context, parent);
                    },
                    child: SizedBox(
                        height: 80,
                        width: 320,
                        child: FlipText(
                          key: Key(titleFlipKey),
                          flipKey: titleFlipKey,
                          text: parent.title,
                          scale: 6,
                          alpha: 80,
                        ))),
              )
            ]
          : [],
    );

    if (parent != _lastParent) {
      _lastParent = parent;
      _disposeAnimations();
      _resetAnimations();

      int itemIndex = slibings?.indexWhere((route) => route == item) ?? -1;

      slibings?.asMap().entries.forEach((entry) {
        final index = entry.key;
        final isCurrent = itemIndex == index;

        if (!isCurrent) {
          final delay = ((index - itemIndex - 1).abs() * 100);
          _slibingOpacities.add(0);

          final timer = Timer(Duration(milliseconds: delay), () {
            if (mounted) {
              setState(() {
                _slibingOpacities[index] = 1;
              });
            }
          });
          _slibingAnimationFutures.add(timer);
        } else {
          _slibingOpacities.add(1);
        }
      });
    }

    final childrenWidget = Row(
      mainAxisAlignment: MainAxisAlignment.start,
      children: slibings?.asMap().entries.map((entry) {
            final route = entry.value;
            final childFlipKey = 'child:${route.path}';

            return Padding(
              padding: const EdgeInsets.only(right: 24),
              child: GestureDetector(
                  onTap: () async {
                    _onRouteSelected(route);
                  },
                  child: AnimatedOpacity(
                    key: Key('animation-$childFlipKey'),
                    opacity: _slibingOpacities[entry.key],
                    duration: const Duration(milliseconds: 300),
                    child: FlipText(
                      key: Key(childFlipKey),
                      flipKey: childFlipKey,
                      text: route.title,
                      scale: 1.2,
                      alpha: route == item ? 255 : 100,
                    ),
                  )),
            );
          }).toList() ??
          [],
    );

    final isSearch = path == '/search';

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
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(
                  child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  parentWidget,
                  Container(
                    padding: const EdgeInsets.symmetric(
                        horizontal: 20, vertical: 12),
                    child: childrenWidget,
                  )
                ],
              )),
              Padding(
                padding: const EdgeInsets.only(top: 54, right: 16),
                child: IconButton(
                    icon: Icon(
                      isSearch ? Symbols.close : Symbols.search,
                      size: 24,
                    ),
                    onPressed: () => {
                          if (isSearch)
                            {
                              if (context.canPop()) {context.pop()}
                            }
                          else
                            {context.push('/search')}
                        }),
              )
            ],
          ),
        ));
  }
}
