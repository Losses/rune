import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../utils/l10n.dart';
import '../../utils/api/complex_query.dart';
import '../../utils/router/navigation.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../config/animation.dart';
import '../../widgets/collection_item.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/start_screen/link_tile.dart';
import '../../widgets/start_screen/start_group.dart';
import '../../widgets/start_screen/utils/group.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';
import '../../widgets/start_screen/constants/default_gap_size.dart';
import '../../widgets/start_screen/start_group_implementation.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../bindings/bindings.dart';
import '../../providers/library_home.dart';

import './constants/first_column.dart';

const groupTitleLinks = {
  "artists": "artists",
  "albums": "albums",
  "playlists": "playlists",
  "tracks": "tracks",
};

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

  late LibraryHomeProvider libraryHome;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    Localizations.localeOf(context);
    libraryHome = Provider.of<LibraryHomeProvider>(context);

    updateLibrary(libraryHome);
  }

  @override
  dispose() {
    super.dispose();
    contextController.dispose();
  }

  final contextController = RouterAwareFlyoutController();
  final contextAttachKey = GlobalKey();

  void openStartScreenContextMenu(
    Offset localPosition,
  ) async {
    if (!context.mounted) return;
    final targetContext = contextAttachKey.currentContext;

    if (targetContext == null) return;
    final box = targetContext.findRenderObject() as RenderBox;
    final position = box.localToGlobal(
      localPosition,
      ancestor: Navigator.of(context).context.findRenderObject(),
    );

    contextController.showFlyout(
      position: position,
      builder: (context) {
        return MenuFlyout(
          items: [
            MenuFlyoutItem(
              leading: const Icon(Symbols.refresh),
              text: Text(S.of(context).refresh),
              onPressed: () async {
                updateLibrary(libraryHome);
              },
            ),
            MenuFlyoutItem(
              leading: const Icon(Symbols.palette),
              text: Text(S.of(context).personalize),
              onPressed: () async {
                $push("/settings/library_home");
              },
            ),
          ],
        );
      },
    );
  }

  void updateLibrary(LibraryHomeProvider libraryHome) {
    setState(() {
      summary = fetchSummary(libraryHome);
    });
  }

  Future<List<Group<InternalCollection>>> fetchSummary(
    LibraryHomeProvider libraryHome,
  ) async {
    widget.layoutManager.resetAnimations();
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
            groupLink: groupTitleLinks[x.id],
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
          return ContextMenuWrapper(
            contextAttachKey: contextAttachKey,
            contextController: contextController,
            onContextMenu: (offset) {
              openStartScreenContextMenu(offset);
            },
            onMiddleClick: (_) {},
            child: LayoutBuilder(
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
                    builder: (context, scrollController) =>
                        SingleChildScrollView(
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
                            onTitleTap: null,
                            itemBuilder: (context, item) {
                              return LinkTile(
                                title: item.$1(context),
                                path: item.$2,
                                icon: item.$3,
                              );
                            },
                            direction: Axis.horizontal,
                          ),
                          ...snapshot.data!
                              .where((x) => x.items.isNotEmpty)
                              .map(
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
                                  onTitleTap: item.groupLink != null
                                      ? () => $push('/${item.groupLink}')
                                      : null,
                                  itemBuilder: (context, item) {
                                    return CollectionItem(
                                      collectionType: item.collectionType,
                                      collection: item,
                                      refreshList: () {},
                                    );
                                  },
                                  direction: Axis.horizontal,
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
            ),
          );
        }
      },
    );
  }
}
