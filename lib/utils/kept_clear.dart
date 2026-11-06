import 'package:flutter/widgets.dart';

/// The system insets Rune keeps its content clear of: the status bar, a notch
/// and the sides of the window. The bottom is left out: content and the
/// playback controller run under the home indicator.
///
/// Fluent's own dialogs, flyouts and info bars keep to the whole safe area
/// with a [SafeArea], the bottom included. They are left so on purpose.
EdgeInsets keptClearOf(EdgeInsets viewPadding) =>
    viewPadding.copyWith(bottom: 0);

extension KeptClear on BuildContext {
  /// The insets of the [MediaQuery] here that content keeps clear of, by
  /// [keptClearOf].
  EdgeInsets get keptClear => keptClearOf(MediaQuery.viewPaddingOf(this));
}
