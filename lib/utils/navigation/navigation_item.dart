import 'package:fluent_ui/fluent_ui.dart';

class NavigationItem {
  final String Function(BuildContext) titleBuilder;
  final String path;
  final bool hidden;
  final void Function(BuildContext)? onTap;
  final bool zuneOnly;
  final List<NavigationItem>? children;
  final List<SingleActivator>? shortcuts;

  /// Tells the item's link in the header's row from the others, and keeps it
  /// from one page to the next, where [path] cannot: a link to a section of an
  /// artist has the page's path while its section is shown.
  final String? linkKey;

  NavigationItem(
    this.titleBuilder,
    this.path, {
    this.hidden = false,
    this.onTap,
    this.children = const [],
    this.zuneOnly = false,
    this.shortcuts,
    this.linkKey,
  });

  @override
  String toString() {
    return 'NavigationItem(title: $titleBuilder, path: $path, hidden: $hidden)';
  }
}
