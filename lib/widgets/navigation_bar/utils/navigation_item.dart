class NavigationItem {
  final String title;
  final String path;
  final bool hidden;
  final bool tappable;
  final List<NavigationItem>? children;

  NavigationItem(this.title, this.path,
      {this.hidden = false, this.tappable = true, this.children = const []});

  @override
  String toString() {
    return 'NavigationItem(title: $title, path: $path, hidden: $hidden)';
  }
}
