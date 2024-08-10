import 'dart:io';
import 'dart:async';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:visibility_detector/visibility_detector.dart';

import '../utils/cover_art_cache.dart';

final coverArtCache = CoverArtCache();

class EmptyCoverArt extends StatelessWidget {
  final double? size;

  const EmptyCoverArt({super.key, this.size});

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      color: Colors.green,
      child: const Icon(Symbols.album),
    );
  }
}

class CoverArt extends StatefulWidget {
  final int? fileId;
  final int? coverArtId;
  final double? size;

  const CoverArt({super.key, this.fileId, this.coverArtId, this.size})
      : assert(fileId != null || coverArtId != null,
            'Either fileId or coverArtId must be provided');

  @override
  CoverArtState createState() => CoverArtState();
}

class CoverArtState extends State<CoverArt> {
  File? _coverArt;

  @override
  void initState() {
    super.initState();
    _loadCoverArt();
  }

  Future<void> _loadCoverArt() async {
    File? cachedCoverArt = await coverArtCache.requestCoverArt(
      fileId: widget.fileId,
      coverArtId: widget.coverArtId,
    );

    if (mounted) {
      setState(() {
        _coverArt = cachedCoverArt;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return VisibilityDetector(
        key: Key('cover-art-${widget.fileId ?? widget.coverArtId}'),
        onVisibilityChanged: (visibilityInfo) {
          if (visibilityInfo.visibleFraction > 0 && _coverArt == null) {
            _loadCoverArt();
          }
        },
        child: _coverArt == null
            ? EmptyCoverArt(size: widget.size ?? double.infinity)
            : Image.file(_coverArt!,
                width: widget.size ?? double.infinity,
                height: widget.size ?? double.infinity,
                fit: BoxFit.cover));
  }
}
