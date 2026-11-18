import '../../bindings/bindings.dart';
import '../../utils/api/request_failure.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';
import '../../providers/library_open_state.dart';
import '../../providers/request_capabilities.dart';
import '../capability_gate.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/playing_item.dart';
import '../../utils/api/get_liked.dart';
import '../../utils/api/set_liked.dart';

import '../rune_clickable.dart';

class LikeButton extends StatefulWidget {
  final PlayingItem? item;
  final Future<bool> Function(PlayingItem item)? fetchLikedStatus;
  final Future<bool> Function(PlayingItem item, bool liked)? setLikedStatus;

  const LikeButton({
    required this.item,
    this.fetchLikedStatus,
    this.setLikedStatus,
    super.key,
  });

  @override
  State<LikeButton> createState() => _LikeButtonState();
}

class _LikeButtonState extends State<LikeButton> {
  bool liked = false;
  late PlayingItem? item;
  int _requestGeneration = 0;
  final Map<PlayingItem, int> _pendingWrites = {};

  PlayingItem? get _likeableItem {
    final current = item;
    return current?.independentFile == null ? current : null;
  }

  @override
  void initState() {
    super.initState();
    item = widget.item;
    _fetchLikedStatus();
  }

  @override
  void didUpdateWidget(covariant LikeButton oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.item != widget.item) {
      item = widget.item;
      liked = false;
      _fetchLikedStatus();
    }
  }

  Future<void> _fetchLikedStatus() async {
    final generation = ++_requestGeneration;
    final requestItem = _likeableItem;
    if (requestItem == null) return;

    try {
      final isLiked =
          await (widget.fetchLikedStatus?.call(requestItem) ??
              getLiked(requestItem));
      if (mounted && generation == _requestGeneration) {
        setState(() {
          liked = isLiked;
        });
      }
    } on RequestFailure catch (error) {
      if (error.kind != RequestErrorKind.notFound &&
          error.kind != RequestErrorKind.unavailable) {
        rethrow;
      }
    }
  }

  Future<void> _toggleLikedStatus() async {
    final generation = ++_requestGeneration;
    final requestItem = _likeableItem;
    if (requestItem == null) return;

    _pendingWrites[requestItem] = (_pendingWrites[requestItem] ?? 0) + 1;
    late bool newLikedStatus;
    try {
      newLikedStatus = await (widget.setLikedStatus ?? setLiked)(
        requestItem,
        !liked,
      );
    } finally {
      final remaining = _pendingWrites[requestItem]! - 1;
      if (remaining == 0) {
        _pendingWrites.remove(requestItem);
      } else {
        _pendingWrites[requestItem] = remaining;
      }
    }
    if (!mounted) return;
    if (generation == _requestGeneration) {
      setState(() {
        liked = newLikedStatus;
      });
    } else if (item == requestItem &&
        !_pendingWrites.containsKey(requestItem)) {
      // A write may have committed after the read made on returning to this item.
      _fetchLikedStatus();
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_likeableItem == null ||
        context.watch<LibraryOpenStateProvider>().isReadOnlyReplica) {
      return const SizedBox.shrink();
    }
    return CapabilityGate(
      capability: capabilityForRequest(
        context,
        (capabilities) => capabilities.setLikedRequest,
      ),
      child: RuneClickable(
        onPressed: _toggleLikedStatus,
        iconSize: 13,
        padding: 0,
        isTiny: true,
        child: Icon(Symbols.favorite, fill: liked ? 1 : 0),
      ),
    );
  }
}
