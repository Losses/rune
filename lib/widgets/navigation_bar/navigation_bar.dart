import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:material_symbols_icons/symbols.dart';

import './flip_text.dart';
import './flip_animation_manager.dart';
import './utils/navigation_item.dart';
import './utils/navigation_query.dart';

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

    final Widget parentWidget = parent != null
        ? Padding(
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
                ),
              ),
            ),
          )
        : Container();

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
      child: Stack(
        children: [
          Transform.translate(
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
          ),
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
                  {
                    if (context.canPop()) {context.pop()}
                  }
                else
                  {context.push('/search')}
              },
            ),
          ),
        ],
      ),
    );
  }
}
