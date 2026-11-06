import 'package:fluent_ui/fluent_ui.dart';

import '../utils/kept_clear.dart';

/// Pads its child clear of the system insets ([keptClearOf]) on the chosen
/// sides, and takes those sides out of both the padding and the view padding
/// below it, so that lists and fluent's scrollbars there do not add them
/// again.
///
/// Wrap only what is in front: a background stays full-bleed.
class KeepClear extends StatelessWidget {
  const KeepClear({
    super.key,
    this.left = true,
    this.top = true,
    this.right = true,
    required this.child,
  });

  final bool left;
  final bool top;
  final bool right;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final media = MediaQuery.of(context);
    final clear = keptClearOf(media.viewPadding);

    EdgeInsets cleared(EdgeInsets insets) => insets.copyWith(
      left: left ? 0 : null,
      top: top ? 0 : null,
      right: right ? 0 : null,
    );

    return Padding(
      padding: EdgeInsets.only(
        left: left ? clear.left : 0,
        top: top ? clear.top : 0,
        right: right ? clear.right : 0,
      ),
      child: MediaQuery(
        data: media.copyWith(
          padding: cleared(media.padding),
          viewPadding: cleared(media.viewPadding),
        ),
        child: child,
      ),
    );
  }
}
