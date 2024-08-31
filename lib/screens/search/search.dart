import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

class SearchPage extends StatefulWidget {
  const SearchPage({super.key});

  @override
  State<SearchPage> createState() => _SearchPageState();
}

const searchCategories = ['Artists', 'Albums', 'Playlists', 'Tracks'];
const searchIcons = [
  Symbols.face,
  Symbols.album,
  Symbols.queue_music,
  Symbols.music_note
];

class _SearchPageState extends State<SearchPage> {
  final searchKey = GlobalKey(debugLabel: 'Search Bar Key');
  final searchFocusNode = FocusNode();
  final searchController = TextEditingController();

  String selectedItem = '';

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    final search = AutoSuggestBox(
      key: searchKey,
      focusNode: searchFocusNode,
      controller: searchController,
      unfocusedColor: Colors.transparent,
      items: <PaneItem>[].map((item) {
        assert(item.title is Text);
        final text = (item.title as Text).data!;
        return AutoSuggestBoxItem(
          label: text,
          value: text,
          onSelected: () {
            item.onTap?.call();
            searchController.clear();
            searchFocusNode.unfocus();
          },
        );
      }).toList(),
      trailingIcon: IgnorePointer(
        child: IconButton(
          onPressed: () {},
          icon: const Icon(FluentIcons.search),
        ),
      ),
    );

    return Row(children: [
      Expanded(child: Container()),
      SizedBox(
        width: 320,
        child: Container(
          color: theme.cardColor,
          child: Padding(
              padding: const EdgeInsets.all(12),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.start,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Padding(
                    padding: const EdgeInsets.symmetric(
                        vertical: 13, horizontal: 16),
                    child: Text("Search", style: typography.bodyLarge),
                  ),
                  Padding(
                    padding:
                        const EdgeInsets.symmetric(vertical: 8, horizontal: 16),
                    child: SizedBox(
                        height: 36,
                        child: Row(
                          mainAxisSize: MainAxisSize.max,
                          children: [
                            Flexible(fit: FlexFit.loose, child: search)
                          ],
                        )),
                  ),
                  const SizedBox(height: 12),
                  Expanded(
                      child: ListView.builder(
                          itemCount: searchCategories.length,
                          itemBuilder: (context, index) {
                            final item = searchCategories[index];
                            return ListTile.selectable(
                              leading: SizedBox(
                                height: 36,
                                child: AspectRatio(
                                  aspectRatio: 1,
                                  child: ColoredBox(
                                    color: theme.accentColor,
                                    child: Icon(searchIcons[index], size: 26),
                                  ),
                                ),
                              ),
                              title: Text(item, style: typography.body),
                              selectionMode: ListTileSelectionMode.single,
                              selected: selectedItem == item,
                              onSelectionChange: (v) =>
                                  setState(() => selectedItem = item),
                            );
                          })),
                ],
              )),
        ),
      ),
    ]);
  }
}
