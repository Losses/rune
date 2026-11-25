class Group<T> {
  final String groupTitle;
  final String? groupLink;
  final List<T> items;

  /// Whether the library fills this group itself, so no collection can be put
  /// in it.
  final bool reserved;

  Group({
    required this.groupTitle,
    required this.items,
    this.groupLink,
    this.reserved = false,
  });
}
