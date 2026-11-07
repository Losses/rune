import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

/// The system insets Rune keeps its content clear of: the status bar, a notch
/// and the sides of the window, and the bottom everywhere but on iOS.
///
/// On iOS the bottom is the home indicator, which is no control: content and
/// the playback controller run under it. On Android it is the navigation bar,
/// whose buttons take the touches there, so content stays above it.
/// [platform] defaults to [defaultTargetPlatform].
///
/// Fluent's own dialogs, flyouts and info bars keep to the whole safe area
/// with a [SafeArea], the bottom included. They are left so on purpose.
EdgeInsets keptClearOf(EdgeInsets viewPadding, {TargetPlatform? platform}) =>
    (platform ?? defaultTargetPlatform) == TargetPlatform.iOS
    ? viewPadding.copyWith(bottom: 0)
    : viewPadding;

extension KeptClear on BuildContext {
  /// The insets of the [MediaQuery] here that content keeps clear of, by
  /// [keptClearOf].
  ///
  /// Read outside a page's content frame, these are the system's insets.
  /// Inside one, the frame has already kept clear of some of them and the
  /// view padding there is only what it leaves to its content (fluent's
  /// scrollbars pad themselves by the view padding, so the frame narrows it
  /// too), so this is what is still to keep clear of there.
  EdgeInsets get keptClear => keptClearOf(MediaQuery.viewPaddingOf(this));
}
