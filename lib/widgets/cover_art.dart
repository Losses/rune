import 'dart:async';
import 'dart:typed_data';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:visibility_detector/visibility_detector.dart';

import '../messages/cover_art.pb.dart';
import '../utils/cover_art_cache.dart';

final coverArtCache = CoverArtCache();

class EmptyCoverArt extends StatelessWidget {
  const EmptyCoverArt({super.key});

  @override
  Widget build(BuildContext context) {
    return Container(
      width: 24,
      height: 24,
      color: Colors.green,
      child: const Icon(Symbols.album),
    );
  }
}

class CoverArt extends StatefulWidget {
  final int fileId;

  const CoverArt({super.key, required this.fileId});

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
    if (cachedCoverArt != null) {
      setState(() {
        _coverArt = cachedCoverArt;
      });
    }
  }

  void _requestCoverArt() {
    if (!_hasRequested && _coverArt == null) {
      CoverArtRequest(fileId: widget.fileId).sendSignalToRust(); // GENERATED
      _hasRequested = true;
    }
  }

  void _listenToCoverArtResponse() {
    _subscription = CoverArtResponse.rustSignalStream.listen((event) async {
      final response = event.message;
      if (response.fileId == widget.fileId) {
        if (mounted) {
          final coverArtData = Uint8List.fromList(response.coverArt);
          await coverArtCache.saveCoverArt(widget.fileId, coverArtData);
          setState(() {
            _coverArt = coverArtData;
          });
        }
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
          ? Container(
              width: 24,
              height: 24,
              color: Colors.magenta,
            )
          : _coverArt!.isEmpty
              ? const EmptyCoverArt()
              : Image.memory(_coverArt!, width: 24, height: 24),
    );
  }
}