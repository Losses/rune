import '../../utils/api/request_failure.dart';
import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:fast_file_picker/fast_file_picker.dart';
import 'package:file_selector/file_selector.dart' show XTypeGroup;

import '../../providers/library_open_state.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';
import 'package:very_good_infinite_list/very_good_infinite_list.dart';

import '../../utils/l10n.dart';
import '../../utils/dialogs/mix/mix_studio.dart';
import '../../utils/dialogs/show_group_list_dialog.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../utils/dialogs/playlist/import_m3u8_playlist.dart';
import '../../utils/dialogs/playlist/create_edit_playlist.dart';
import '../../config/animation.dart';
import '../../widgets/no_items.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../screens/collection/utils/is_user_generated.dart';
import '../../screens/collection/utils/collection_item_builder.dart';
import '../../screens/collection/utils/collection_data_provider.dart';
import '../../bindings/bindings.dart';

import '../infinite_list_loading.dart';
import '../smooth_horizontal_scroll.dart';
import '../navigation_bar/page_content_frame.dart';

import 'utils/group.dart';
import 'utils/internal_collection.dart';
import 'providers/start_screen_layout_manager.dart';

import 'start_group.dart';
import '../../utils/dialogs/file_dialog/pick_files.dart';

class StartScreen extends StatelessWidget {
  const StartScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        return StartScreenImplementation(constraints: constraints);
      },
    );
  }
}

class StartScreenImplementation extends StatefulWidget {
  final BoxConstraints constraints;

  const StartScreenImplementation({super.key, required this.constraints});

  @override
  StartScreenImplementationState createState() =>
      StartScreenImplementationState();
}

class StartScreenImplementationState extends State<StartScreenImplementation>
    with
        SingleTickerProviderStateMixin,
        CollectionGroupNavigation<StartScreenImplementation> {
  final _layoutManager = StartScreenLayoutManager();
  late final _scrollController = SmoothScrollController(vsync: this);

  final _contextController = RouterAwareFlyoutController();
  final _contextAttachKey = GlobalKey();

  @override
  void dispose() {
    _scrollController.dispose();
    _layoutManager.dispose();
    _contextController.dispose();
    super.dispose();
  }

  BoxConstraints _constraintsWithin(EdgeInsets padding) {
    final c = widget.constraints;
    return BoxConstraints(
      maxWidth: c.maxWidth - padding.left - padding.right,
      maxHeight: c.maxHeight - padding.top - padding.bottom,
    );
  }

  StartGroup<InternalCollection> _buildGroup(
    BuildContext context,
    CollectionDataProvider data,
    int index,
    BoxConstraints constraints,
  ) {
    final item = data.items[index];
    final isUserGenerated = userGenerated(data.collectionType);
    return StartGroup<InternalCollection>(
      key: ValueKey(item.groupTitle),
      groupIndex: index,
      groupTitle: item.groupTitle,
      items: item.items,
      constraints: constraints,
      onTitleTap: () {
        if (!isUserGenerated) {
          showGroupListDialog(context, scrollToGroup);
        }
      },
      itemBuilder: (context, item) => collectionItemBuilder(context, item),
      direction: Axis.vertical,
    );
  }

  @override
  void scrollToGroupIndex(CollectionDataProvider data, int index) {
    final trueConstraints = _constraintsWithin(
      getScrollContainerPadding(context, listen: false),
    );
    double scrollPosition = 0.0;

    for (int i = 0; i < index; i++) {
      scrollPosition += _buildGroup(
        context,
        data,
        i,
        trueConstraints,
      ).widthIn(context);
    }

    _scrollController.scrollTo(scrollPosition);
  }

  Future<void> _fetchData({bool retry = false}) async {
    if (!mounted) return;
    final data = Provider.of<CollectionDataProvider>(context, listen: false);
    if (data.pageError != null && !retry) return;
    await data.fetchData();
    if (!mounted || data.pageError != null) return;

    Timer(
      Duration(milliseconds: gridAnimationDelay),
      () => _layoutManager.playAnimations(),
    );
  }

  void openStartScreenContextMenu(Offset localPosition) async {
    if (!context.mounted) return;
    final targetContext = _contextAttachKey.currentContext;

    if (targetContext == null) return;
    final box = targetContext.findRenderObject() as RenderBox;
    final position = box.localToGlobal(
      localPosition,
      ancestor: Navigator.of(context).context.findRenderObject(),
    );

    final data = Provider.of<CollectionDataProvider>(context, listen: false);

    _contextController.showFlyout(
      position: position,
      builder: (_) {
        return MenuFlyout(
          items: [
            MenuFlyoutItem(
              leading: const Icon(Symbols.refresh),
              text: Text(S.of(context).refresh),
              onPressed: () async {
                await _layoutManager.replayAfter(data.reloadData());
              },
            ),
            if (!context.read<LibraryOpenStateProvider>().isReadOnlyReplica &&
                data.collectionType == CollectionType.mix)
              MenuFlyoutItem(
                leading: const Icon(Symbols.add),
                text: Text(S.of(context).newMix),
                onPressed: () async {
                  final x = await showMixStudioDialog(context);

                  if (x != null) data.reloadData();
                },
              ),
            if (!context.read<LibraryOpenStateProvider>().isReadOnlyReplica &&
                data.collectionType == CollectionType.playlist)
              MenuFlyoutItem(
                leading: const Icon(Symbols.add),
                text: Text(S.of(context).newPlaylist),
                onPressed: () async {
                  final x = await showCreateEditPlaylistDialog(context, "");

                  if (x != null) data.reloadData();
                },
              ),
            if (!context.read<LibraryOpenStateProvider>().isReadOnlyReplica &&
                data.collectionType == CollectionType.playlist)
              MenuFlyoutItem(
                leading: const Icon(Symbols.download),
                text: Text(S.of(context).importM3u8),
                onPressed: () async {
                  const XTypeGroup typeGroup = XTypeGroup(
                    label: 'playlist',
                    extensions: <String>['m3u', 'm3u8'],
                  );
                  final FastFilePickerPath? file = await pickFile(
                    context,
                    acceptedTypeGroups: <XTypeGroup>[typeGroup],
                  );

                  if (file == null) return;
                  if (!mounted) return;

                  final x = await showCreateImportM3u8PlaylistDialog(
                    context,
                    file,
                  );

                  if (x != null) data.reloadData();
                },
              ),
          ],
        );
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final padding = getScrollContainerPadding(context);
    final trueConstraints = _constraintsWithin(padding);

    final data = Provider.of<CollectionDataProvider>(context);
    final isUserGenerated = userGenerated(data.collectionType);
    final pageError = data.pageError;

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: _layoutManager,
      child: FutureBuilder<List<Group<InternalCollection>>>(
        future: data.summary,
        builder: (context, snapshot) {
          // A failed summary keeps its error while the next one loads, so the
          // spinner also waits for the items: those that stayed are shown.
          if (snapshot.connectionState == ConnectionState.waiting &&
              !snapshot.hasData &&
              data.items.isEmpty) {
            return const Center(child: ProgressRing());
          } else if (snapshot.hasError && data.items.isEmpty) {
            // Only with nothing on screen: a reload whose summary fails keeps
            // its items.
            return Center(child: Text(requestErrorText(S.of(context), snapshot.error!)));
          } else {
            return ContextMenuWrapper(
              contextAttachKey: _contextAttachKey,
              contextController: _contextController,
              onContextMenu: (offset) {
                openStartScreenContextMenu(offset);
              },
              onMiddleClick: (_) {},
              child: SmoothHorizontalScroll(
                controller: _scrollController,
                builder: (context, smoothScrollController) {
                  return InfiniteList(
                    itemCount: data.items.length,
                    scrollDirection: Axis.horizontal,
                    scrollController: smoothScrollController,
                    loadingBuilder: (context) => const InfiniteListLoading(),
                    centerLoading: true,
                    centerEmpty: true,
                    isLoading: data.isLoading,
                    hasError: pageError != null,
                    centerError: data.items.isEmpty,
                    errorBuilder: (_) => InfiniteListRetry(
                      error: pageError!,
                      onRetry: () => retryGroupOrPage(() => _fetchData(retry: true)),
                    ),
                    padding: padding,
                    emptyBuilder: (context) => Center(
                      child: data.initialized
                          ? NoItems(
                              title: S.of(context).noCollectionFound,
                              hasRecommendation: false,
                              reloadData: data.reloadData,
                              userGenerated: isUserGenerated,
                            )
                          : Container(),
                    ),
                    onFetchData: _fetchData,
                    hasReachedMax: data.isLastPage,
                    itemBuilder: (context, index) {
                      return _buildGroup(
                        context,
                        data,
                        index,
                        trueConstraints,
                      );
                    },
                  );
                },
              ),
            );
          }
        },
      ),
    );
  }
}
