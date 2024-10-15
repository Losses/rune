import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/navigation/navigation_item.dart';
import '../../utils/navigation/utils/escape_from_search.dart';
import '../../config/navigation_query.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../providers/responsive_providers.dart';

import './flip_text.dart';
import './flip_animation_manager.dart';

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

  List<Timer> _slibingAnimationFutures = [];
  List<double> _slibingOpacities = [];
  NavigationItem? _lastParent;
  bool? _lastIsZune;

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
    return SmallerOrEqualTo(
      breakpoint: DeviceType.zune,
      builder: (context, isZune) {
        final path = GoRouterState.of(context).fullPath;
        final item = navigationQuery.getItem(path, isZune);
        final parent = navigationQuery.getParent(path, isZune);
        final slibings = navigationQuery
            .getSiblings(path, isZune)
            ?.where((x) => !x.hidden)
            .toList();

        final titleFlipKey = 'title:${parent?.path}';

        final Widget parentWidget = parent != null
            ? Padding(
                padding: const EdgeInsets.only(right: 12),
                child: SizedBox(
                  height: 80,
                  width: 320,
                  child: ParentText(
                    titleFlipKey: titleFlipKey,
                    text: parent.title,
                    onTap: () => _onHeaderTap(context, parent),
                  ),
                ),
              )
            : Container();

        final baseSlibings = (slibings ?? emptySlibings);
        final validSlibings = isZune
            ? baseSlibings
            : baseSlibings.where((x) => !x.zuneOnly).toList();

        if (parent != _lastParent || isZune != _lastIsZune) {
          _lastParent = parent;
          _lastIsZune = isZune;
          _disposeAnimations();
          _resetAnimations();

          int itemIndex = validSlibings.indexWhere((route) => route == item);

          validSlibings.asMap().entries.forEach((entry) {
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

        final childrenWidget = SmoothHorizontalScroll(
          builder: (context, scrollController) => SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            controller: scrollController,
            padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.start,
              children: validSlibings.toList().asMap().entries.map(
                (entry) {
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
                      ),
                    ),
                  );
                },
              ).toList(),
            ),
          ),
        );

        final isSearch = path == '/search';

        return Stack(
          children: [
            if (isZune || !isSearch)
              Transform.translate(
                offset: const Offset(0, -40),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  mainAxisSize: MainAxisSize.max,
                  children: [
                    parentWidget,
                    childrenWidget,
                  ],
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

class ParentText extends StatefulWidget {
  final String titleFlipKey;
  final String text;
  final VoidCallback onTap;

  const ParentText({
    super.key,
    required this.titleFlipKey,
    required this.text,
    required this.onTap,
  });

  @override
  ParentTextState createState() => ParentTextState();
}

class ParentTextState extends State<ParentText> {
  double _alpha = 80;
  double _glowRadius = 0;
  final FocusNode _focusNode = FocusNode();

  @override
  void initState() {
    super.initState();
    _focusNode.addListener(_handleFocusChange);
  }

  @override
  void dispose() {
    _focusNode.removeListener(_handleFocusChange);
    _focusNode.dispose();
    super.dispose();
  }

  void _handleFocusChange() {
    setState(() {
      _glowRadius = _focusNode.hasFocus ? 20 : 0;
    });
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(right: 12),
      child: GestureDetector(
        onTap: widget.onTap,
        child: Focus(
          focusNode: _focusNode,
          child: MouseRegion(
            onEnter: (_) {
              setState(() {
                _alpha = 100;
              });
            },
            onExit: (_) {
              setState(() {
                _alpha = 80;
              });
            },
            child: SizedBox(
              height: 80,
              width: 320,
              child: FlipText(
                key: Key(widget.titleFlipKey),
                flipKey: widget.titleFlipKey,
                text: widget.text,
                scale: 6,
                alpha: _alpha,
                glowColor: Colors.red,
                glowRadius: _glowRadius,
              ),
            ),
          ),
        ),
      ),
    );
  }
}
