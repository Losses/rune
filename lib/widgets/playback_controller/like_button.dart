import 'dart:async';

import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import '../../utils/api/request_failure.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';
import '../../providers/library_open_state.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/playing_item_capabilities.dart';
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
  // Whether the item's library answers about its likes, as it last declared.
  bool? _answers;
  StreamSubscription<RustSignalPack<SetLikedResponse>>? _likesSet;

  PlayingItem? get _likeableItem {
    final current = item;
    return current?.independentFile == null ? current : null;
  }

  @override
  void initState() {
    super.initState();
    item = widget.item;
    _likesSet = SetLikedResponse.rustSignalStream.listen(_onLikeSet);
    _fetchLikedStatus();
  }

  @override
  void dispose() {
    _likesSet?.cancel();
    super.dispose();
  }

  /// A like set anywhere, as from a menu, shows here when it is of the item
  /// shown. This button's own writes show when they return.
  void _onLikeSet(RustSignalPack<SetLikedResponse> signal) {
    final current = _likeableItem;
    if (current == null ||
        _pendingWrites.containsKey(current) ||
        PlayingItem.fromRequest(signal.message.item) != current) {
      return;
    }
    ++_requestGeneration;
    if (mounted) setState(() => liked = signal.message.liked);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final answers = _answersAboutLikes(listen: true);
    // A library that starts answering, as once its service signs in, is asked
    // anew.
    if (_answers == false && answers) _fetchLikedStatus();
    _answers = answers;
  }

  @override
  void didUpdateWidget(covariant LikeButton oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.item != widget.item) {
      item = widget.item;
      liked = false;
      _answers = _answersAboutLikes(listen: false);
      _fetchLikedStatus();
    }
  }

  /// What the item's library declares about [select] for the item.
  RequestCapability _itemCapability(
    RequestCapability Function(PlayingItemCapabilities capabilities) select, {
    required bool listen,
  }) =>
      Provider.of<PlayingItemCapabilitiesProvider?>(
        context,
        listen: listen,
      )?.capabilityFor(item, select) ??
      const RequestCapabilityAvailable();

  bool _answersAboutLikes({required bool listen}) =>
      _itemCapability(
            (capabilities) => capabilities.getLikedRequest,
            listen: listen,
          )
          is RequestCapabilityAvailable;

  Future<void> _fetchLikedStatus() async {
    final generation = ++_requestGeneration;
    final requestItem = _likeableItem;
    if (requestItem == null) return;
    // Until the library answers there is no like to show.
    if (!_answersAboutLikes(listen: false)) {
      if (liked) setState(() => liked = false);
      return;
    }

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
      // A library whose service needs a sign-in cannot say yet: the like is
      // not known, which is no failure to show.
      if (error.needsServiceSignIn) {
        if (mounted && generation == _requestGeneration && liked) {
          setState(() => liked = false);
        }
        return;
      }
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
    // The item's own library decides first: it may not answer for the item
    // whichever library is open, as until its service signs in.
    final itemCapability = _itemCapability(
      (capabilities) => capabilities.setLikedRequest,
      listen: true,
    );
    return CapabilityGate(
      capability: itemCapability is RequestCapabilityAvailable
          ? capabilityForRequest(
              context,
              (capabilities) => capabilities.setLikedRequest,
            )
          : itemCapability,
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
