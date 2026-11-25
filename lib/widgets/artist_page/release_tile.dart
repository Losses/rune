import 'dart:math' as math;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import '../../utils/router/router_aware_flyout_controller.dart';
import '../ax_pressure.dart';
import '../ax_reveal/ax_reveal.dart';
import '../context_menu_wrapper.dart';
import '../start_screen/constants/default_gap_size.dart';
import '../start_screen/start_group.dart' show defaultCellSize;
import '../tile/flip_tile.dart';
import '../tile/rolling_cover_circles.dart';
import '../tile/tile.dart';

/// Opens the context menu of a release tile at [position], which is local to
/// the tile. The menu is shown by [controller] on the target [attachKey].
typedef ReleaseContextMenuOpener =
    void Function(
      BuildContext context,
      Offset position,
      GlobalKey attachKey,
      RouterAwareFlyoutController controller,
    );

/// The tile of one release section: the section's covers in circles that
/// roll across the theme's accent colour, under the section's name. It is
/// named and behaves as a collection tile of the library home, and fills the
/// box it is given.
class ReleaseTile extends StatefulWidget {
  /// The section name, drawn as a collection tile's name is.
  final String label;

  /// Local paths of the section's covers.
  final List<String> covers;

  /// The cells along a side of the tile, 1 or 2. A larger tile has more
  /// circles. Its covers are decoded for the size it is laid out at, or for
  /// its cells and the gaps between them where that is not known.
  final int cells;

  /// How the circles are laid out.
  final CircleArrangementKind arrangement;

  final VoidCallback onPressed;

  /// What a right click, or a long press on a touch screen, does.
  final ReleaseContextMenuOpener? onContextMenu;

  /// What a middle click does.
  final VoidCallback? onMiddleClick;

  const ReleaseTile({
    super.key,
    required this.label,
    required this.covers,
    required this.onPressed,
    this.cells = 1,
    this.arrangement = CircleArrangementKind.square,
    this.onContextMenu,
    this.onMiddleClick,
  });

  @override
  State<ReleaseTile> createState() => _ReleaseTileState();
}

class _ReleaseTileState extends State<ReleaseTile> {
  final _contextController = RouterAwareFlyoutController();
  final _contextAttachKey = GlobalKey();

  @override
  void dispose() {
    _contextController.dispose();
    super.dispose();
  }

  /// The side the covers are decoded for: the tile's own, which differs from
  /// layout to layout.
  int _coverSide(BoxConstraints constraints) {
    final side = math.min(constraints.maxWidth, constraints.maxHeight);
    if (side.isFinite) return side.round();
    return (defaultCellSize * widget.cells +
            defaultGapSize * (widget.cells - 1))
        .round();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final accent = theme.accentColor;
    // Each cover is on one circle at a time, so each is given once.
    final covers = widget.covers.toSet().toList();

    // The same shell as a collection tile on the library home.
    return AxPressure(
      child: AxReveal0(
        child: ContextMenuWrapper(
          contextAttachKey: _contextAttachKey,
          contextController: _contextController,
          onMiddleClick: (_) => widget.onMiddleClick?.call(),
          onContextMenu: (position) => widget.onContextMenu?.call(
            context,
            position,
            _contextAttachKey,
            _contextController,
          ),
          child: Tile(
            onPressed: widget.onPressed,
            child: NamedTileCover(
              name: widget.label,
              child: covers.isNotEmpty
                  ? LayoutBuilder(
                      builder: (context, constraints) => RollingCoverCircles(
                        paths: covers,
                        arrangement: widget.arrangement,
                        cells: widget.cells,
                        size: _coverSide(constraints),
                        background: accent,
                        fillers: [
                          accent.light,
                          accent.lighter,
                          accent.dark,
                          accent.darker,
                        ],
                      ),
                    )
                  : EmptyFlipCover(
                      name: widget.label,
                      emptyTileType: BoringAvatarType.bauhaus,
                      colors: flipTileColors(theme),
                    ),
            ),
          ),
        ),
      ),
    );
  }
}
