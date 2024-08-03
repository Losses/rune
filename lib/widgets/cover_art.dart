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
  final int fileId;
  final double size;

  const CoverArt({super.key, required this.fileId, required this.size});

  @override
  CoverArtState createState() => CoverArtState();
}

class CoverArtState extends State<CoverArt> {
  bool _hasRequested = false;
  Uint8List? _coverArt;
  StreamSubscription? _subscription;

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
    final cachedCoverArt = await coverArtCache.getCoverArt(widget.fileId);
    if (cachedCoverArt != null && mounted) {
      setState(() {
        _coverArt = cachedCoverArt;
      });
    }
  }

  void _requestCoverArt() {
    if (!_hasRequested && _coverArt == null) {
      GetCoverArtByFileIdRequest(fileId: widget.fileId)
          .sendSignalToRust(); // GENERATED
      _hasRequested = true;
    }
  }

  void _listenToCoverArtResponse() {
    _subscription = CoverArtByFileIdResponse.rustSignalStream.listen((event) async {
      final response = event.message;
      if (response.fileId == widget.fileId) {
        if (!mounted) return;

        final coverArtData = Uint8List.fromList(response.coverArt);
        await coverArtCache.saveCoverArt(widget.fileId, coverArtData);
        if (!mounted) return;

        setState(() {
          _coverArt = coverArtData;
        });
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    return VisibilityDetector(
      key: Key('cover-art-${widget.fileId}'),
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
