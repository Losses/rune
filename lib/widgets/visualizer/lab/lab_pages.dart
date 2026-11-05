import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

/// How wide a page handle is across its edge, and so how big it is to touch.
const labPageHandleThickness = 24.0;

/// A release this fast, along the way the pages slide, flips them whichever
/// way they are, if it is toward the other page.
const _flingVelocity = 365.0;

/// The cards page and the timeline page of the lab on a small window, one on
/// the screen at a time. Each has a handle on the edge that faces the other
/// page: a tap on it goes to the other page, and a drag moves both pages
/// with the pointer and lets them go to the nearer one. The pages themselves
/// take no drag, the timeline has its own.
///
/// The page that is not shown stays in the tree, so it keeps its scroll and
/// its zoom, but takes no pointer, is hidden from a screen reader and does
/// not tick.
class LabPages extends StatefulWidget {
  const LabPages({
    super.key,
    required this.axis,
    required this.padding,
    required this.cards,
    required this.timeline,
  });

  /// The way the pages slide: horizontal puts the cards page on the left and
  /// the timeline page on the right, vertical the cards page on top.
  final Axis axis;

  /// The margin around each page's body and handle.
  final EdgeInsets padding;
  final Widget cards;
  final Widget timeline;

  @override
  State<LabPages> createState() => _LabPagesState();
}

class _LabPagesState extends State<LabPages>
    with SingleTickerProviderStateMixin {
  // How far the cards page is in: 0 is the timeline page on the screen, 1 the
  // cards page.
  late final AnimationController _in = AnimationController(
    vsync: this,
    value: 0,
  );

  bool get _horizontal => widget.axis == Axis.horizontal;

  @override
  void dispose() {
    _in.dispose();
    super.dispose();
  }

  void _go(double page) {
    final theme = FluentTheme.of(context);
    _in.animateTo(
      page,
      duration: theme.mediumAnimationDuration,
      curve: theme.animationCurve,
    );
  }

  // Whether a drag of a handle has begun, as a tap also ends the drag
  // recognizer, by its cancel, and that is not a release.
  bool _dragging = false;

  void _dragStart() {
    _dragging = true;
    _in.stop();
  }

  // Moving toward the cards page, right or down, brings it in, whichever
  // handle holds the pointer.
  void _drag(double delta) {
    final size = context.size;
    if (size == null) return;
    final extent = _horizontal ? size.width : size.height;
    _in.value = (_in.value + delta / extent).clamp(0.0, 1.0);
  }

  void _release(double velocity) {
    if (!_dragging) return;
    _dragging = false;
    if (velocity.abs() > _flingVelocity) {
      _go(velocity > 0 ? 1 : 0);
    } else {
      _go(_in.value > 0.5 ? 1 : 0);
    }
  }

  // A tap goes to the page the handle names, [page], not to whichever page
  // the pages are not on, which mid-slide is the one they came from.
  Widget _handle(String label, double page) => LabPageHandle(
    axis: widget.axis,
    label: label,
    onTap: () => _go(page),
    onDragStart: _dragStart,
    onDrag: _drag,
    onRelease: _release,
  );

  // A page: its body and its handle on the edge that faces the other page,
  // named for a screen reader by the page it shows.
  Widget _page({required bool cards, required Widget body}) {
    final handle = _handle(cards ? 'Timeline' : 'Cards', cards ? 0 : 1);
    final children = [Expanded(child: body), handle];
    final ordered = cards ? children : children.reversed.toList();
    final child = Padding(
      padding: widget.padding,
      child: _horizontal
          ? Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: ordered,
            )
          : Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: ordered,
            ),
    );
    return Positioned.fill(
      child: AnimatedBuilder(
        animation: _in,
        child: child,
        builder: (context, child) {
          final shown = cards ? _in.value : 1 - _in.value;
          final off = shown == 0;
          // The cards page leaves toward the start, the timeline page toward
          // the end.
          final away = (1 - shown) * (cards ? -1 : 1);
          return TickerMode(
            enabled: !off,
            child: ExcludeSemantics(
              excluding: off,
              child: IgnorePointer(
                ignoring: off,
                child: FractionalTranslation(
                  translation: _horizontal ? Offset(away, 0) : Offset(0, away),
                  child: child,
                ),
              ),
            ),
          );
        },
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return ClipRect(
      child: Stack(
        children: [
          _page(cards: true, body: widget.cards),
          _page(cards: false, body: widget.timeline),
        ],
      ),
    );
  }
}

/// The handle on a page's edge that faces the other page: a grip on a strip
/// wide enough to touch. A tap shows the other page; a drag moves the pages.
class LabPageHandle extends StatelessWidget {
  const LabPageHandle({
    super.key,
    required this.axis,
    required this.label,
    required this.onTap,
    required this.onDragStart,
    required this.onDrag,
    required this.onRelease,
  });

  /// The way the pages slide.
  final Axis axis;

  /// The page it shows, named on its tooltip.
  final String label;

  final VoidCallback onTap;
  final VoidCallback onDragStart;

  /// Called with how far the pointer moved along [axis].
  final ValueChanged<double> onDrag;

  /// Called with how fast the pointer moved along [axis] as it was let go.
  final ValueChanged<double> onRelease;

  @override
  Widget build(BuildContext context) {
    final horizontal = axis == Axis.horizontal;
    // The grip's dots run up and down; on a handle along the top or bottom
    // they are turned to run across.
    Widget icon = const Icon(Symbols.drag_indicator, size: 16);
    if (!horizontal) icon = RotatedBox(quarterTurns: 1, child: icon);
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        onHorizontalDragStart: horizontal ? (_) => onDragStart() : null,
        onHorizontalDragUpdate: horizontal
            ? (d) => onDrag(d.primaryDelta ?? 0)
            : null,
        onHorizontalDragEnd: horizontal
            ? (d) => onRelease(d.primaryVelocity ?? 0)
            : null,
        onHorizontalDragCancel: horizontal ? () => onRelease(0) : null,
        onVerticalDragStart: horizontal ? null : (_) => onDragStart(),
        onVerticalDragUpdate: horizontal
            ? null
            : (d) => onDrag(d.primaryDelta ?? 0),
        onVerticalDragEnd: horizontal
            ? null
            : (d) => onRelease(d.primaryVelocity ?? 0),
        onVerticalDragCancel: horizontal ? null : () => onRelease(0),
        child: SizedBox(
          width: horizontal ? labPageHandleThickness : null,
          height: horizontal ? null : labPageHandleThickness,
          child: Center(
            child: Tooltip(
              message: label,
              child: IconButton(
                icon: icon,
                // As wide as the strip.
                style: const ButtonStyle(
                  padding: WidgetStatePropertyAll(EdgeInsets.all(4)),
                ),
                onPressed: onTap,
              ),
            ),
          ),
        ),
      ),
    );
  }
}
