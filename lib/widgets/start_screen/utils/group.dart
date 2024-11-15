class Group<T> {
  final String groupTitle;
  final String? groupLink;
  final List<T> items;

  Group({
    required this.groupTitle,
    required this.items,
    this.groupLink,
  });
}
