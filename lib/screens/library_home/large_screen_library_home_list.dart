import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/library_home.dart';

import '../../messages/all.dart';
import '../../utils/api/complex_query.dart';
import '../../utils/l10n.dart';
import '../../utils/router/navigation.dart';
import '../../config/animation.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/start_screen/link_tile.dart';
import '../../widgets/start_screen/start_group.dart';
import '../../widgets/start_screen/utils/group.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';
import '../../widgets/start_screen/constants/default_gap_size.dart';
import '../../widgets/start_screen/start_group_implementation.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../screens/collection/collection_item.dart';

import './constants/first_column.dart';

class LargeScreenLibraryHomeListView extends StatefulWidget {
  final String libraryPath;
  final StartScreenLayoutManager layoutManager;
  final bool topPadding;

  const LargeScreenLibraryHomeListView({
    super.key,
    required this.libraryPath,
    required this.layoutManager,
    required this.topPadding,
  });

  @override
  LibraryHomeListState createState() => LibraryHomeListState();
}

class LibraryHomeListState extends State<LargeScreenLibraryHomeListView> {
  Future<List<Group<dynamic>>>? summary;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    Provider.of<LibraryHomeProvider>(context, listen: false)
        .addListener(updateLibrary);
  }

  @override
  dispose() {
    super.dispose();
    Provider.of<LibraryHomeProvider>(context, listen: false)
        .removeListener(updateLibrary);
  }

  void updateLibrary() {
    final libraryHome =
        Provider.of<LibraryHomeProvider>(context, listen: false);
    setState(() {
      summary = fetchSummary(libraryHome);
    });
  }

  Future<List<Group<InternalCollection>>> fetchSummary(
    LibraryHomeProvider libraryHome,
  ) async {
    final librarySummary = await complexQuery(
      libraryHome.entries
          .where((x) => x.value != null && x.value != 'disable')
          .map(
            (x) => ComplexQuery(
              id: x.id,
              title: x.definition.titleBuilder(context),
              domain: x.definition.id,
              parameter: x.value!,
            ),
          )
          .toList(),
    );

    if (!mounted) return [];
    if (librarySummary == null) return [];

    final groups = librarySummary.result
        .map(
          (x) => Group<InternalCollection>(
            groupTitle: x.title,
            groupLink: "artists",
            items: x.entries
                .map(InternalCollection.fromComplexQueryEntry)
                .toList(),
          ),
        )
        .toList();

    Timer(
      Duration(milliseconds: gridAnimationDelay),
      () => widget.layoutManager.playAnimations(),
    );

    return groups;
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<List<Group<dynamic>>>(
      future: summary,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return Container();
        } else if (snapshot.hasError) {
          return Center(child: Text('Error: ${snapshot.error}'));
        } else if (!snapshot.hasData || snapshot.data!.isEmpty) {
          return Center(child: Text(S.of(context).noDataAvailable));
        } else {
          return LayoutBuilder(
            builder: (context, constraints) {
              final padding = getScrollContainerPadding(context);
              final c = constraints;
              final trueConstraints = BoxConstraints(
                maxWidth: c.maxWidth - padding.left - padding.right,
                maxHeight: c.maxHeight - padding.top - padding.bottom,
              );

              return Container(
                alignment: Alignment.centerLeft,
                child: SmoothHorizontalScroll(
                  builder: (context, scrollController) => SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    controller: scrollController,
                    padding: getScrollContainerPadding(context,
                        top: widget.topPadding),
                    child: Row(
                      mainAxisAlignment: MainAxisAlignment.start,
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        StartGroup<
                            (
                              String Function(BuildContext),
                              String,
                              IconData,
                              bool
                            )>(
                          groupIndex: 0,
                          groupTitle: S.of(context).start,
                          items: smallScreenFirstColumn
                              .where((x) => !x.$4)
                              .toList(),
                          constraints: trueConstraints,
                          groupLayoutVariation:
                              StartGroupGroupLayoutVariation.stacked,
                          gridLayoutVariation:
                              StartGroupGridLayoutVariation.initial,
                          dimensionCalculator: StartGroupImplementation
                              .startLinkDimensionCalculator,
                          gapSize: defaultGapSize,
                          onTitleTap: () {},
                          itemBuilder: (context, item) {
                            return LinkTile(
                              title: item.$1(context),
                              path: item.$2,
                              icon: item.$3,
                            );
                          },
                        ),
                        ...snapshot.data!.map(
                          (item) {
                            if (item is Group<InternalCollection>) {
                              return StartGroup<InternalCollection>(
                                groupIndex: item.groupTitle.hashCode,
                                groupTitle: item.groupTitle,
                                groupLink: item.groupLink,
                                items: item.items,
                                constraints: trueConstraints,
                                groupLayoutVariation:
                                    StartGroupGroupLayoutVariation.stacked,
                                gridLayoutVariation:
                                    StartGroupGridLayoutVariation.square,
                                gapSize: defaultGapSize,
                                onTitleTap: () => $push('/${item.groupLink}'),
                                itemBuilder: (context, item) {
                                  return CollectionItem(
                                    collectionType: item.collectionType,
                                    collection: item,
                                    refreshList: () {},
                                  );
                                },
                              );
                            } else {
                              return Container();
                            }
                          },
                        )
                      ],
                    ),
                  ),
                ),
              );
            },
          );
        }
      },
    );
  }
}
