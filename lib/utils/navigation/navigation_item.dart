import 'package:fluent_ui/fluent_ui.dart';

class NavigationItem {
  final String Function(BuildContext) titleBuilder;
  final String path;
  final bool hidden;
  final void Function(BuildContext)? onTap;
  final bool zuneOnly;
  final List<NavigationItem>? children;
  final List<SingleActivator>? shortcuts;

  NavigationItem(
    this.titleBuilder,
    this.path, {
    this.hidden = false,
    this.onTap,
    this.children = const [],
    this.zuneOnly = false,
    this.shortcuts,
  });

  @override
  String toString() {
    return 'NavigationItem(title: $titleBuilder, path: $path, hidden: $hidden)';
  }
}
