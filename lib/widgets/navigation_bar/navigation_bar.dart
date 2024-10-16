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
                  child: ParentLink(
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

        final int currentItemIndex =
            validSlibings.indexWhere((route) => route == item);

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

class SlibingLink extends StatefulWidget {
  final NavigationItem route;
  final bool isSelected;
  final int? delay;
  final VoidCallback onTap;

  const SlibingLink({
    super.key,
    required this.route,
    required this.isSelected,
    required this.delay,
    required this.onTap,
  });

  @override
  State<SlibingLink> createState() => _SlibingLinkState();
}

class _SlibingLinkState extends State<SlibingLink> {
  Timer? timer;
  late double _entryAnimationOpacity;

  bool _isHovered = false;
  double _glowRadius = 0;

  @override
  void initState() {
    super.initState();
    final delay = widget.delay;
    if (delay != null) {
      timer = Timer(Duration(milliseconds: delay), () {
        if (!mounted) return;

        setState(() {
          _entryAnimationOpacity = 1;
        });
      });
      _entryAnimationOpacity = 0;
    } else {
      _entryAnimationOpacity = 1;
    }
  }

  @override
  void dispose() {
    super.dispose();

    timer?.cancel();
  }

  void _handleFocusHighlight(bool value) {
    setState(() {
      _glowRadius = value ? 20 : 0;
    });
  }

  void _handleHoveHighlight(bool value) {
    setState(() {
      _isHovered = value;
    });
  }

  @override
  Widget build(BuildContext context) {
    final childFlipKey = 'child:${widget.route.path}';

    return Padding(
      padding: const EdgeInsets.only(right: 24),
      child: GestureDetector(
        onTap: widget.onTap,
        child: FocusableActionDetector(
          onShowFocusHighlight: _handleFocusHighlight,
          onShowHoverHighlight: _handleHoveHighlight,
          child: AnimatedOpacity(
            key: Key('animation-$childFlipKey'),
            opacity: _entryAnimationOpacity,
            duration: const Duration(milliseconds: 300),
            child: FlipText(
              key: Key(childFlipKey),
              flipKey: childFlipKey,
              text: widget.route.title,
              scale: 1.2,
              glowColor: Colors.red,
              glowRadius: _glowRadius,
              alpha: widget.isSelected
                  ? 255
                  : _isHovered
                      ? 200
                      : 100,
            ),
          ),
        ),
      ),
    );
  }
}

class ParentLink extends StatefulWidget {
  final String titleFlipKey;
  final String text;
  final VoidCallback onTap;

  const ParentLink({
    super.key,
    required this.titleFlipKey,
    required this.text,
    required this.onTap,
  });

  @override
  ParentLinkState createState() => ParentLinkState();
}

class ParentLinkState extends State<ParentLink> {
  double _alpha = 80;
  double _glowRadius = 0;

  void _handleFocusHighlight(bool value) {
    setState(() {
      _glowRadius = value ? 20 : 0;
    });
  }

  void _handleHoveHighlight(bool value) {
    setState(() {
      _alpha = value ? 100 : 80;
    });
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(right: 12),
      child: GestureDetector(
        onTap: widget.onTap,
        child: FocusableActionDetector(
          onShowFocusHighlight: _handleFocusHighlight,
          onShowHoverHighlight: _handleHoveHighlight,
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
    );
  }
}
