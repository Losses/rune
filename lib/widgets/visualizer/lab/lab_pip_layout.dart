import 'package:flutter/widgets.dart';

/// The side of the overlay the floating picture is tucked into, if any.
enum LabPipSide { none, left, right }

/// Where the floating picture rests and how wide it is: what is kept between
/// two openings of the lab.
@immutable
class LabPipLayout {
  const LabPipLayout({
    this.corner = Alignment.bottomRight,
    this.width,
    this.stash = LabPipSide.none,
    this.stashAt = 0.5,
  });

  /// The corner it rests in when it is not tucked away.
  final Alignment corner;

  /// Its width; none for the default.
  final double? width;

  final LabPipSide stash;

  /// Where the tab sits on the side it is tucked into: 0 at the top of the
  /// room it has, 1 at the bottom. A part rather than pixels, so that it
  /// survives a turn of the window.
  final double stashAt;

  LabPipLayout withWidth(double? width) => LabPipLayout(
    corner: corner,
    width: width,
    stash: stash,
    stashAt: stashAt,
  );

  /// Out in [corner], keeping the width.
  LabPipLayout inCorner(Alignment corner) =>
      LabPipLayout(corner: corner, width: width);

  /// Tucked into [side] at [at], keeping the corner it came from.
  LabPipLayout tucked(LabPipSide side, double at) =>
      LabPipLayout(corner: corner, width: width, stash: side, stashAt: at);

  @override
  bool operator ==(Object other) =>
      other is LabPipLayout &&
      other.corner == corner &&
      other.width == width &&
      other.stash == stash &&
      other.stashAt == stashAt;

  @override
  int get hashCode => Object.hash(corner, width, stash, stashAt);
}

/// The names the corners are saved under.
const labPipCornerNames = <String, Alignment>{
  'top_left': Alignment.topLeft,
  'top_right': Alignment.topRight,
  'bottom_left': Alignment.bottomLeft,
  'bottom_right': Alignment.bottomRight,
};

String labPipCornerName(Alignment corner) => labPipCornerNames.entries
    .firstWhere(
      (e) => e.value == corner,
      orElse: () => const MapEntry('bottom_right', Alignment.bottomRight),
    )
    .key;
