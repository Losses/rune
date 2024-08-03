import 'dart:async';
import 'dart:typed_data';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:visibility_detector/visibility_detector.dart';

import '../messages/cover_art.pb.dart';
import '../utils/cover_art_cache.dart';

final coverArtCache = CoverArtCache();

class EmptyCoverArt extends StatelessWidget {
  final double size;

  const EmptyCoverArt({super.key, required this.size});

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
  final double size;

  const CoverArt({super.key, this.fileId, this.coverArtId, required this.size})
      : assert(fileId != null || coverArtId != null,
            'Either fileId or coverArtId must be provided');

  @override
  CoverArtState createState() => CoverArtState();
}

class CoverArtState extends State<CoverArt> {
  bool _hasRequested = false;
  Uint8List? _coverArt;
  StreamSubscription? _subscription;

  late int? coverArtId = widget.coverArtId;

  @override
  void initState() {
    super.initState();
    _checkCache();
    _listenToCoverArtResponse();
  }

  @override
  void dispose() {
    _subscription?.cancel();
    super.dispose();
  }

  Future<void> _checkCache() async {
    Uint8List? cachedCoverArt;
    if (widget.coverArtId != null) {
      cachedCoverArt = await coverArtCache.getCoverArt(widget.coverArtId!);
    }

    if (cachedCoverArt != null && mounted) {
      setState(() {
        _coverArt = cachedCoverArt;
      });
    }
  }

  void _requestCoverArt() {
    if (!_hasRequested && _coverArt == null) {
      if (widget.fileId != null) {
        GetCoverArtByFileIdRequest(fileId: widget.fileId!).sendSignalToRust();
      } else if (widget.coverArtId != null) {
        GetCoverArtByCoverArtIdRequest(coverArtId: widget.coverArtId!)
            .sendSignalToRust();
      }
      _hasRequested = true;
    }
  }

  void _listenToCoverArtResponse() {
    _subscription =
        CoverArtByFileIdResponse.rustSignalStream.listen((event) async {
      final response = event.message;
      if (widget.fileId != null && response.fileId == widget.fileId) {
        coverArtId = response.coverArtId;
        _handleCoverArtResponse(response.coverArt, widget.fileId);
      }
    });

    _subscription =
        CoverArtByCoverArtIdResponse.rustSignalStream.listen((event) async {
      final response = event.message;
      if (widget.coverArtId != null &&
          response.coverArtId == widget.coverArtId) {
        _handleCoverArtResponse(response.coverArt, widget.coverArtId);
      }
    });
  }

  Future<void> _handleCoverArtResponse(List<int>? coverArt, int? id) async {
    if (!mounted) return;

    final coverArtData =
        coverArt != null ? Uint8List.fromList(coverArt) : Uint8List(0);
    await coverArtCache.saveCoverArt(coverArtId, coverArtData);

    if (!mounted) return;

    setState(() {
      _coverArt = coverArtData;
    });
  }

  @override
  Widget build(BuildContext context) {
    return VisibilityDetector(
      key: Key('cover-art-${widget.fileId ?? widget.coverArtId}'),
      onVisibilityChanged: (visibilityInfo) {
        if (visibilityInfo.visibleFraction > 0 && _coverArt == null) {
          _requestCoverArt();
        }
      },
      child: _coverArt == null
          ? SizedBox(
              width: widget.size,
              height: widget.size,
            )
          : _coverArt!.isEmpty
              ? EmptyCoverArt(size: widget.size)
              : Image.memory(_coverArt!,
                  width: widget.size, height: widget.size),
    );
  }
}
