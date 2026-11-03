import '../../bindings/bindings.dart';
import '../../utils/api/request_failure.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/playing_item.dart';
import '../../utils/api/get_liked.dart';
import '../../utils/api/set_liked.dart';

import '../rune_clickable.dart';

class LikeButton extends StatefulWidget {
  final PlayingItem? item;

  const LikeButton({required this.item, super.key});

  @override
  State<LikeButton> createState() => _LikeButtonState();
}

class _LikeButtonState extends State<LikeButton> {
  bool liked = false;
  late PlayingItem? item;

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
      _fetchLikedStatus();
    }
  }

  Future<void> _fetchLikedStatus() async {
    if (item == null) return;

    try {
      final isLiked = await getLiked(item!);
      if (mounted) {
        setState(() {
          liked = isLiked;
        });
      }
    } on RequestFailure catch (error) {
      if (error.kind != RequestErrorKind.notFound) rethrow;
    }
  }

  Future<void> _toggleLikedStatus() async {
    if (item == null) return;

    final newLikedStatus = !liked;
    try {
      final success = await setLiked(item!, newLikedStatus);
      if (success != null && mounted) {
        setState(() {
          liked = newLikedStatus;
        });
      }
    } on RequestFailure catch (error) {
      if (error.kind != RequestErrorKind.notFound) rethrow;
    }
  }

  @override
  Widget build(BuildContext context) {
    return RuneClickable(
      onPressed: item == null
          ? null
          : () {
              _toggleLikedStatus();
            },
      iconSize: 13,
      padding: 0,
      isTiny: true,
      child: Icon(
        Symbols.favorite,
        fill: liked ? 1 : 0,
      ),
    );
  }
}
