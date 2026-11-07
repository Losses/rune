import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../providers/responsive_providers.dart';

/// How the lab lays out its cards and its timeline in a window.
enum LabLayout {
  /// The cards above the timeline, on one page.
  together,

  /// One page at a time, the cards beside the timeline: a phone on its side.
  pagesAcross,

  /// One page at a time, the cards above the timeline: a phone upright.
  pagesDown;

  /// Whether the cards and the timeline are pages of their own.
  bool get paged => this != together;
}

/// The least height of a window that has room for the cards above the
/// timeline under the playback controller. A phone on its side is shorter,
/// and wide enough to count as a desktop by its width alone.
const labTogetherMinHeight = 560.0;

/// How the lab lays out in a [window] of this size: together where the app
/// has a desktop or a tablet layout and the window is tall enough, else in
/// pages, across the window or down it as it is wider or taller.
LabLayout labLayoutFor(Size window) {
  final narrow = window.width <= DeviceType.mobile.end;
  final short = window.height < labTogetherMinHeight;
  if (!narrow && !short) return LabLayout.together;
  return window.height > window.width
      ? LabLayout.pagesDown
      : LabLayout.pagesAcross;
}

/// How the lab lays out in the window [context] is in, by the size layouts
/// are chosen by ([ScreenSizeProvider.layoutSize]).
LabLayout labLayoutOf(BuildContext context, {bool listen = true}) =>
    labLayoutFor(
      Provider.of<ScreenSizeProvider>(context, listen: listen).layoutSize,
    );
