import '../../api/request_failure.dart';
import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/responsive_dialog_actions.dart';
import '../../../widgets/track_list/utils/internal_media_file.dart';
import '../../../widgets/start_screen/managed_start_screen_item.dart';
import '../../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../../widgets/navigation_bar/constants/navigation_bar_height.dart';
import '../../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../../screens/search/widgets/track_search_item.dart';
import '../../../bindings/bindings.dart';
import '../../../providers/library_registry.dart';
import '../../../providers/responsive_providers.dart';

import '../../query_list.dart';
import '../../api/create_mix.dart';
import '../../api/update_mix.dart';
import '../../api/get_mix_by_id.dart';
import '../../api/query_mix_tracks.dart';
import '../../api/fetch_mix_queries_by_mix_id.dart';
import '../../dialogs/mix/widgets/mix_editor.dart';
import '../../dialogs/mix/utils/mix_editor_data.dart';
import '../../dialogs/mix/widgets/mix_editor_controller.dart';

import '../unavailable_dialog_on_band.dart';

class MixStudioDialog extends StatefulWidget {
  final int? mixId;
  final void Function(Mix?) $close;

  const MixStudioDialog({super.key, required this.mixId, required this.$close});

  @override
  State<MixStudioDialog> createState() => _MixStudioDialog();
}

class _MixStudioDialog extends State<MixStudioDialog> {
  @override
  Widget build(BuildContext context) {
    return MixStudioDialogImplementation(
      mixId: widget.mixId,
      $close: widget.$close,
    );
  }
}

class MixStudioDialogImplementation extends StatefulWidget {
  final int? mixId;
  final void Function(Mix?) $close;

  const MixStudioDialogImplementation({
    super.key,
    required this.mixId,
    required this.$close,
  });

  @override
  State<MixStudioDialogImplementation> createState() =>
      _MixStudioDialogImplementationState();
}

class _MixStudioDialogImplementationState
    extends State<MixStudioDialogImplementation> {
  late final _controller = MixEditorController();
  final _layoutManager = StartScreenLayoutManager();
  final _scrollController = ScrollController();
  final _editorKey = GlobalKey();

  bool isLoading = false;
  Object? _initialLoadError;
  Object? _previewError;
  Object? _saveError;
  int _previewGeneration = 0;
  String _query = '';
  int _cursor = 0;
  bool _isLoadingMore = false;
  bool _hasMore = true;
  static const int _pageSize = 100;
  List<InternalMediaFile> _allTracks = [];
  Timer? _debounceTimer;
  bool _isInitialLoad = true;
  late final LibraryRegistryProvider? _library;
  late final String? _libraryId;
  late bool _hasDraft;

  bool get _inOriginalLibrary =>
      _library == null ||
      (_libraryId != null &&
          _library.openRecordId == _libraryId &&
          !_library.isOpening);

  @override
  void initState() {
    super.initState();

    _library = context.read<LibraryRegistryProvider?>();
    _libraryId = _library?.openRecordId;
    _hasDraft = widget.mixId == null;
    _scrollController.addListener(_onScroll);

    _controller.addListener(() {
      // Skip the first listener call during loadMix
      if (_isInitialLoad) {
        _isInitialLoad = false;
        return;
      }
      _debouncedResetAndLoad();
    });
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _loadInitialMixIfReady();
  }

  void _loadInitialMixIfReady() {
    if (_hasDraft ||
        isLoading ||
        _initialLoadError != null ||
        !_inOriginalLibrary) {
      return;
    }
    loadMix(widget.mixId!);
  }

  void _debouncedResetAndLoad() {
    // A response for the old draft is stale even while the new query is debouncing.
    _previewGeneration++;
    _isLoadingMore = false;
    _debounceTimer?.cancel();
    _debounceTimer = Timer(const Duration(milliseconds: 300), () {
      _resetAndLoadTracks();
    });
  }

  void _onScroll() {
    if (_scrollController.position.pixels >=
            _scrollController.position.maxScrollExtent - 200 &&
        _previewError == null &&
        _debounceTimer?.isActive != true &&
        !_isLoadingMore &&
        _hasMore) {
      _loadMoreTracks();
    }
  }

  Future<void> _resetAndLoadTracks() async {
    if (!_inOriginalLibrary) return;
    final generation = ++_previewGeneration;
    final libraryGeneration = _library?.openGeneration;
    setState(() => _isLoadingMore = true);
    _layoutManager.resetAnimations();

    // Don't clear the tracks immediately to avoid flickering
    // Instead, load new data first, then replace
    final query = mixEditorDataToQuery(_controller.getData());
    final newQuery = query.map((x) => '$x').join(';');

    try {
      final newTracks = await queryMixTracks(QueryList(query), 0, _pageSize);
      if (!_acceptPreview(generation, libraryGeneration)) return;
      setState(() {
        _previewError = null;
        _query = newQuery;
        _cursor = newTracks.length;
        _hasMore = newTracks.length >= _pageSize;
        _allTracks = newTracks;
        _isLoadingMore = false;
      });

      // Reset scroll position to top for new results
      if (_scrollController.hasClients) {
        _scrollController.jumpTo(0);
      }

      _layoutManager.playAnimations();
    } on Exception catch (error) {
      if (!_acceptPreview(generation, libraryGeneration)) return;
      setState(() {
        _previewError = error;
        _query = newQuery;
        _cursor = 0;
        _hasMore = false;
        _allTracks = [];
        _isLoadingMore = false;
      });
    }
  }

  Future<void> _loadMoreTracks() async {
    if (_isLoadingMore || !_hasMore || !_inOriginalLibrary) return;
    final generation = _previewGeneration;
    final libraryGeneration = _library?.openGeneration;
    setState(() {
      _isLoadingMore = true;
    });

    try {
      final query = mixEditorDataToQuery(_controller.getData());
      final newTracks = await queryMixTracks(
        QueryList(query),
        _cursor,
        _pageSize,
      );

      if (!_acceptPreview(generation, libraryGeneration)) return;
      setState(() {
        _previewError = null;
        _allTracks.addAll(newTracks);
        _cursor += newTracks.length;
        _hasMore = newTracks.length >= _pageSize;
        _isLoadingMore = false;
      });
    } on Exception catch (error) {
      if (!_acceptPreview(generation, libraryGeneration)) return;
      setState(() {
        _previewError = error;
        _isLoadingMore = false;
      });
    }
  }

  bool _acceptPreview(int generation, int? libraryGeneration) {
    if (!mounted || generation != _previewGeneration) return false;
    if (_inOriginalLibrary && _library?.openGeneration == libraryGeneration) {
      return true;
    }
    setState(() => _isLoadingMore = false);
    return false;
  }

  Future<void> _retryPreview() {
    if (_cursor == 0 || _debounceTimer?.isActive == true) {
      _debounceTimer?.cancel();
      return _resetAndLoadTracks();
    }
    return _loadMoreTracks();
  }

  Future<void> loadMix(int mixId) async {
    final libraryGeneration = _library?.openGeneration;
    bool sameRead() =>
        _inOriginalLibrary && _library?.openGeneration == libraryGeneration;
    setState(() {
      isLoading = true;
      _initialLoadError = null;
    });
    try {
      final mix = await getMixById(mixId);
      final queries = await fetchMixQueriesByMixId(mixId);
      final data = await queryToMixEditorData(mix, queries);
      if (!mounted) return;
      if (!sameRead()) return;
      _controller.setData(data);
      _hasDraft = true;
      await _resetAndLoadTracks();
    } catch (error) {
      if (!mounted) return;
      if (sameRead()) {
        setState(() => _initialLoadError = error);
      }
    } finally {
      if (mounted) {
        setState(() => isLoading = false);
        _loadInitialMixIfReady();
      }
    }
  }

  @override
  void dispose() {
    _debounceTimer?.cancel();
    _scrollController.dispose();
    _controller.dispose();
    _layoutManager.dispose();
    super.dispose();
  }

  Future<void> saveMix() async {
    if (!_canSave) return;
    setState(() {
      isLoading = true;
      _saveError = null;
    });

    final editorData = _controller.getData();
    final queries = mixEditorDataToQuery(editorData);

    Mix? response;
    try {
      if (widget.mixId != null) {
        response = await updateMix(
          widget.mixId!,
          _controller.titleController.text,
          _controller.groupController.text,
          editorData.mode,
          queries,
        );
      } else {
        response = await createMix(
          _controller.titleController.text,
          _controller.groupController.text,
          editorData.mode,
          queries,
        );
      }
    } on Exception catch (error) {
      if (mounted) {
        setState(() {
          _saveError = error;
          isLoading = false;
        });
      }
      return;
    }
    if (mounted) widget.$close(response);
  }

  bool get _canSave =>
      !isLoading &&
      _hasDraft &&
      _initialLoadError == null &&
      _inOriginalLibrary;

  @override
  Widget build(BuildContext context) {
    context.watch<LibraryRegistryProvider?>();
    final r = Provider.of<ResponsiveProvider>(context);
    final height = MediaQuery.of(context).size.height;
    const reduce = fullNavigationBarHeight + playbackControllerHeight + 48;

    final smallerThanTablet = r.smallerOrEqualTo(DeviceType.tablet);
    final smallerThanZune = r.smallerOrEqualTo(DeviceType.zune);

    final editorHeight =
        (height.clamp(dialogMinimumViewportHeight, double.infinity) - reduce)
            .clamp(120.0, double.infinity);
    String errorMessage(Object error) => error is RequestFailure
        ? requestFailureMessage(S.of(context), error)
        : error.toString();
    final initialError = _initialLoadError;
    final editor = SizedBox(
      height: editorHeight,
      child: initialError != null
          ? Center(child: Text(errorMessage(initialError)))
          : !_hasDraft
          ? Center(
              child: _inOriginalLibrary
                  ? const ProgressRing()
                  : Text(S.of(context).requestUnavailable),
            )
          : Column(
              children: [
                if (!_inOriginalLibrary) Text(S.of(context).requestUnavailable),
                if (_saveError case final error?) Text(errorMessage(error)),
                if (_previewError case final error?)
                  Row(
                    children: [
                      Expanded(child: Text(errorMessage(error))),
                      Button(
                        onPressed: _isLoadingMore || !_inOriginalLibrary
                            ? null
                            : _retryPreview,
                        child: Text(S.of(context).playOnRetry),
                      ),
                    ],
                  ),
                Expanded(
                  child: MixEditor(key: _editorKey, controller: _controller),
                ),
              ],
            ),
    );

    return UnavailableDialogOnBand(
      $close: widget.$close,
      child: NoShortcuts(
        ContentDialog(
          style: smallerThanZune
              ? const ContentDialogThemeData(
                  padding: EdgeInsets.symmetric(vertical: 24, horizontal: 8),
                )
              : null,
          constraints: BoxConstraints(maxWidth: smallerThanTablet ? 420 : 1000),
          title: Column(
            children: [
              const SizedBox(height: 8),
              Text(
                widget.mixId != null
                    ? S.of(context).editMix
                    : S.of(context).createMix,
              ),
            ],
          ),
          content: Container(
            constraints: BoxConstraints(maxHeight: editorHeight),
            child: smallerThanTablet
                ? editor
                : Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      SizedBox(width: 380, child: editor),
                      const SizedBox(width: 6),
                      Expanded(
                        child: SizedBox(
                          height: editorHeight,
                          child:
                              ChangeNotifierProvider<
                                StartScreenLayoutManager
                              >.value(
                                value: _layoutManager,
                                child: LayoutBuilder(
                                  builder: (context, constraints) {
                                    const double gapSize = 8;
                                    const double cellSize = 200;

                                    const ratio = 4 / 1;

                                    final int rows =
                                        (constraints.maxWidth /
                                                (cellSize + gapSize))
                                            .floor();

                                    final trackIds = _allTracks
                                        .map((x) => x.id)
                                        .toList();

                                    return GridView(
                                      controller: _scrollController,
                                      gridDelegate:
                                          SliverGridDelegateWithFixedCrossAxisCount(
                                            crossAxisCount: rows,
                                            mainAxisSpacing: gapSize,
                                            crossAxisSpacing: gapSize,
                                            childAspectRatio: ratio,
                                          ),
                                      children: _allTracks
                                          .map(
                                            (a) => TrackSearchItem(
                                              index: 0,
                                              item: a,
                                              fallbackFileIds: trackIds,
                                            ),
                                          )
                                          .toList()
                                          .asMap()
                                          .entries
                                          .map((x) {
                                            final index = x.key;
                                            final int row = index % rows;
                                            final int column = index ~/ rows;

                                            return ManagedStartScreenItem(
                                              key: Key('$row:$column'),
                                              prefix: _query,
                                              groupId: 0,
                                              row: row,
                                              column: column,
                                              width: cellSize / ratio,
                                              height: cellSize,
                                              child: x.value,
                                            );
                                          })
                                          .toList(),
                                    );
                                  },
                                ),
                              ),
                        ),
                      ),
                    ],
                  ),
          ),
          actions: [
            ResponsiveDialogActions(
              FilledButton(
                onPressed: _canSave ? saveMix : null,
                child: Text(
                  widget.mixId != null
                      ? S.of(context).save
                      : S.of(context).create,
                ),
              ),
              Button(
                onPressed: isLoading ? null : () => widget.$close(null),
                child: Text(S.of(context).cancel),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
