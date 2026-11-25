List<String> cleanGroupTitles(List<String> titles) {
  final uniqueTitles = <String>{};

  for (final title in titles) {
    final cleanTitle = title.replaceAll('\u{200B}', '');
    if (cleanTitle != "Rune") {
      uniqueTitles.add(cleanTitle);
    }
  }

  return uniqueTitles.toList();
}
