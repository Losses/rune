import '../../utils/api/request_failure.dart';
import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../utils/l10n.dart';
import '../../utils/api/complex_query.dart';
import '../../utils/router/navigation.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
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
import '../../providers/request_capabilities.dart';
import '../settings_library_home/constants/library_home_items.dart';

import './constants/first_column.dart';

const groupTitleLinks = <LibraryHomeDomain, String>{
  LibraryHomeDomain.artists: "artists",
  LibraryHomeDomain.albums: "albums",
  LibraryHomeDomain.playlists: "playlists",
  LibraryHomeDomain.mixes: "mixes",
  LibraryHomeDomain.tracks: "tracks",
};

class LargeScreenLibraryHomeListView extends StatefulWidget {
  final StartScreenLayoutManager layoutManager;

  /// Loads the summaries of the home's query sections. Tests replace it.
  final Future<ComplexQueryResponse?> Function(List<ComplexQuery> queries)
  loadSummary;

  const LargeScreenLibraryHomeListView({
    super.key,
    required this.layoutManager,
    this.loadSummary = complexQuery,
  });

  @override
  LibraryHomeListState createState() => LibraryHomeListState();
}

class LibraryHomeListState extends State<LargeScreenLibraryHomeListView> {
  Future<List<Group<dynamic>>>? summary;

  late LibraryHomeProvider libraryHome;
  bool _offersSummary = true;
  StreamSubscription? _contentSubscription;

  @override
  void initState() {
    super.initState();
    _contentSubscription = LibraryContentChanged.rustSignalStream.listen((_) {
      if (mounted) updateLibrary(libraryHome);
    });
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    Localizations.localeOf(context);
    libraryHome = Provider.of<LibraryHomeProvider>(context);
    _offersSummary =
        capabilityForRequest(
              context,
              (capabilities) => capabilities.complexQueryRequest,
            )
            is RequestCapabilityAvailable;

    updateLibrary(libraryHome);
  }

  @override
  dispose() {
    _contentSubscription?.cancel();
    contextController.dispose();
    super.dispose();
  }

  final contextController = RouterAwareFlyoutController();
  final contextAttachKey = GlobalKey();

  void openStartScreenContextMenu(Offset localPosition) async {
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
    // Until the home's entries are read there is nothing to ask for, and an empty answer
    // would read as no data.
    if (!libraryHome.loaded) return;
    setState(() {
      summary = fetchSummary(libraryHome);
    });
  }

  Future<List<Group<InternalCollection>>> fetchSummary(
    LibraryHomeProvider libraryHome,
  ) async {
    var groups = <Group<InternalCollection>>[];
    if (_offersSummary) {
      final librarySummary = await widget.layoutManager.replayAfter(
        widget.loadSummary(
          libraryHome.entries
              .where((x) => x.setting is LibraryHomeSettingQuery)
              .map((x) {
                final setting = x.setting as LibraryHomeSettingQuery;
                return ComplexQuery(
                  title: x.definition.titleBuilder(context),
                  query: setting.query,
                );
              })
              .toList(),
        ),
      );

      if (!mounted) return [];
      if (librarySummary == null) return [];

      groups = librarySummary.result
          .map(
            (x) => Group<InternalCollection>(
              groupTitle: x.title,
              groupLink: groupTitleLinks[x.domain],
              items: x.entries
                  .map(InternalCollection.fromComplexQueryEntry)
                  .toList(),
            ),
          )
          .toList();
    } else {
      // The Start group's tiles appear only once the animations play, so a
      // library without summaries plays them too.
      await widget.layoutManager.replayAfter(Future<void>.value());
    }

    return groups;
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<List<Group<dynamic>>>(
      future: summary,
      builder: (context, snapshot) {
        if (summary == null ||
            (snapshot.connectionState == ConnectionState.waiting &&
                !snapshot.hasData)) {
          return const Center(child: ProgressRing());
        } else if (snapshot.hasError) {
          final error = snapshot.error;
          return Center(
            child: Text(
              error is RequestFailure
                  ? requestFailureMessage(S.of(context), error)
                  : 'Error: $error',
            ),
          );
        } else if (_offersSummary &&
            (!snapshot.hasData || snapshot.data!.isEmpty)) {
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
                          padding: getScrollContainerPadding(context),
                          child: Row(
                            mainAxisAlignment: MainAxisAlignment.start,
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              StartGroup<
                                (
                                  String Function(BuildContext),
                                  String,
                                  IconData,
                                  bool,
                                )
                              >(
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
                              ...(snapshot.data ?? const [])
                                  .where((x) => x.items.isNotEmpty)
                                  .map((item) {
                                    if (item is Group<InternalCollection>) {
                                      return StartGroup<InternalCollection>(
                                        groupIndex: item.groupTitle.hashCode,
                                        groupTitle: item.groupTitle,
                                        groupLink: item.groupLink,
                                        items: item.items,
                                        constraints: trueConstraints,
                                        groupLayoutVariation:
                                            StartGroupGroupLayoutVariation
                                                .stacked,
                                        gridLayoutVariation:
                                            StartGroupGridLayoutVariation
                                                .square,
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
                                  }),
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
