import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/slide_fade_transition.dart';
import '../../../screens/search/widgets/search_card.dart';
import '../../../messages/search.pb.dart';
import '../../../messages/collection.pbenum.dart';
import '../../../generated/l10n.dart';

import '../constants/search_icons.dart';
import '../constants/search_categories.dart';

import './search_suggest_box.dart';

class LargeScreenSearchSidebar extends StatelessWidget {
  final CollectionType selectedItem;
  final SearchSuggestBox autoSuggestBox;
  final SearchForResponse? searchResults;
  final void Function(CollectionType) setSelectedField;

  final Map<CollectionType, List<SearchCard>> items;

  const LargeScreenSearchSidebar({
    super.key,
    required this.selectedItem,
    required this.autoSuggestBox,
    required this.searchResults,
    required this.setSelectedField,
    required this.items,
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    return SizedBox(
      width: 320,
      child: SlideFadeTransition(
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
                    vertical: 13,
                    horizontal: 16,
                  ),
                  child:
                      Text(S.of(context).search, style: typography.bodyLarge),
                ),
                Padding(
                  padding: const EdgeInsets.symmetric(
                    vertical: 8,
                    horizontal: 16,
                  ),
                  child: SizedBox(
                    height: 36,
                    child: Row(
                      mainAxisSize: MainAxisSize.max,
                      children: [
                        Flexible(fit: FlexFit.loose, child: autoSuggestBox)
                      ],
                    ),
                  ),
                ),
                const SizedBox(height: 12),
                Expanded(
                  child: ListView.builder(
                    itemCount: searchCategories.length,
                    itemBuilder: (context, index) {
                      final item = searchCategories[index];
                      final int itemCount = items[item]?.length ?? 0;

                      return ListTile.selectable(
                        leading: Container(
                          width: 36,
                          height: 36,
                          decoration: BoxDecoration(
                            color: theme.accentColor,
                            borderRadius: BorderRadius.circular(2),
                          ),
                          child: Icon(
                            searchIcons[index],
                            color: theme.activeColor,
                            size: 26,
                          ),
                        ),
                        title: Row(
                          children: [
                            Expanded(
                              child: Text(
                                '${item.toString()}s',
                                style: typography.body,
                              ),
                            ),
                            if (itemCount > 0)
                              Text(
                                '$itemCount',
                                style: typography.body!.copyWith(
                                  color: theme.inactiveColor.withAlpha(160),
                                ),
                              ),
                          ],
                        ),
                        selectionMode: ListTileSelectionMode.single,
                        selected: selectedItem == item,
                        onSelectionChange: (v) {
                          setSelectedField(item);
                        },
                      );
                    },
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
