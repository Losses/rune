import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/api/get_liked.dart';
import '../../utils/api/set_liked.dart';
import '../rune_icon_button.dart';

class LikeButton extends StatefulWidget {
  final int? fileId;

  const LikeButton({required this.fileId, super.key});

  @override
  State<LikeButton> createState() => _LikeButtonState();
}

class _LikeButtonState extends State<LikeButton> {
  bool liked = false;
  late int? fileId;

  @override
  void initState() {
    super.initState();
    fileId = widget.fileId;
    _fetchLikedStatus();
  }

  @override
  void didUpdateWidget(covariant LikeButton oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.fileId != widget.fileId) {
      fileId = widget.fileId;
      _fetchLikedStatus();
    }
  }

  Future<void> _fetchLikedStatus() async {
    if (fileId == null) return;

    final isLiked = await getLiked(fileId!);
    if (mounted) {
      setState(() {
        liked = isLiked;
      });
    }
  }

  Future<void> _toggleLikedStatus() async {
    if (fileId == null) return;

    final newLikedStatus = !liked;
    final success = await setLiked(fileId!, newLikedStatus);
    if (success != null && mounted) {
      setState(() {
        liked = newLikedStatus;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return RuneIconButton(
      onPressed: fileId == null
          ? null
          : () {
              _toggleLikedStatus();
            },
      icon: Icon(
        Symbols.favorite,
        fill: liked ? 1 : 0,
      ),
      iconSize: 13,
      padding: 0,
      isTiny: true,
    );
  }
}
