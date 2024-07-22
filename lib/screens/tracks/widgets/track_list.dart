import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../../messages/media_file.pb.dart';
import '../../../messages/playback.pb.dart';

class TrackListView extends StatefulWidget {
  const TrackListView({super.key});

  @override
  TrackListViewState createState() => TrackListViewState();
}

class TrackListViewState extends State<TrackListView> {
  static const _pageSize = 20;

  final PagingController<int, MediaFile> _pagingController =
      PagingController(firstPageKey: 0);

  @override
  void initState() {
    _pagingController.addPageRequestListener((pageKey) {
      _fetchPage(pageKey);
    });
    super.initState();
  }

  Future<void> _fetchPage(int pageKey) async {
    try {
      final fetchMediaFiles = FetchMediaFiles(
        pageKey: pageKey,
        pageSize: _pageSize,
      );
      fetchMediaFiles.sendSignalToRust(); // GENERATED

      // Listen for the response from Rust
      final rustSignal = await MediaFileList.rustSignalStream.first;
      final mediaFileList = rustSignal.message;
      final newItems = mediaFileList.mediaFiles;

      final isLastPage = newItems.length < _pageSize;
      if (isLastPage) {
        _pagingController.appendLastPage(newItems);
      } else {
        final nextPageKey = pageKey + newItems.length;
        _pagingController.appendPage(newItems, nextPageKey);
      }
    } catch (error) {
      _pagingController.error = error;
    }
  }

  @override
  Widget build(BuildContext context) => PagedListView<int, MediaFile>(
        pagingController: _pagingController,
        builderDelegate: PagedChildBuilderDelegate<MediaFile>(
          itemBuilder: (context, item, index) => ListTile.selectable(
              title: Text(item.path),
              onSelectionChange: (v) => PlayFileRequest(fileId: item.id)
                  .sendSignalToRust() // GENERATED,
              ),
        ),
      );

  @override
  void dispose() {
    _pagingController.dispose();
    super.dispose();
  }
}
