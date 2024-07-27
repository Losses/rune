import 'dart:typed_data';
import 'package:flutter/material.dart';

import '../messages/cover_art.pb.dart';

class CoverArt extends StatefulWidget {
  final int fileId;

  const CoverArt({super.key, required this.fileId});

  @override
  CoverArtState createState() => CoverArtState();
}

class CoverArtState extends State<CoverArt> {
  late Stream<CoverArtResponse> _coverArtStream;

  @override
  void initState() {
    super.initState();
    _coverArtStream = CoverArtResponse.rustSignalStream
        as Stream<CoverArtResponse>; // GENERATED
    _requestCoverArt();
  }

  void _requestCoverArt() {
    CoverArtRequest(fileId: widget.fileId).sendSignalToRust(); // GENERATED
  }

  @override
  Widget build(BuildContext context) {
    return StreamBuilder<CoverArtResponse>(
      stream: _coverArtStream,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return const CircularProgressIndicator();
        } else if (snapshot.hasError) {
          return Text('Error: ${snapshot.error}');
        } else if (!snapshot.hasData ||
            snapshot.data!.fileId != widget.fileId) {
          return const Text('No cover art available');
        } else {
          final coverArt = snapshot.data!.coverArt;
          final coverArtBytes = Uint8List.fromList(coverArt);
          return Image.memory(coverArtBytes);
        }
      },
    );
  }
}
