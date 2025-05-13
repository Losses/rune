import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/nearest_power_of_two.dart';
import '../../utils/process_cover_art_path.dart';
import '../../widgets/tile/fancy_cover.dart';

class EmptyCoverArt extends StatelessWidget {
  final double? size;
  final int index;

  const EmptyCoverArt({
    super.key,
    this.size,
    required this.index,
  });

  @override
  Widget build(BuildContext context) {
    final accentColor = FluentTheme.of(context).accentColor;

    final colors = [
      accentColor.dark,
      accentColor.darker,
      accentColor.darkest,
      accentColor.normal,
      accentColor.light,
      accentColor.lighter,
      accentColor.lightest
    ];

    final colorIndex = index % colors.length;

    return Container(
      width: size,
      height: size,
      color: colors[colorIndex],
    );
  }
}

class CoverArt extends StatefulWidget {
  final String? path;
  final (String, String, String)? hint;
  final double? size;
  final int hash;

  const CoverArt({
    super.key,
    required this.path,
    this.size,
    this.hint,
    this.hash = 0,
  });

  @override
  State<CoverArt> createState() => _CoverArtState();
}

class _CoverArtState extends State<CoverArt> {
  late Future<String> _processedPathFuture;

  @override
  void initState() {
    super.initState();
    _processPath();
  }

  @override
  void didUpdateWidget(CoverArt oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.path != widget.path) {
      _processPath();
    }
  }

  void _processPath() {
    _processedPathFuture = widget.path != null && widget.path!.isNotEmpty
        ? processCoverArtPath(widget.path!)
        : Future.value(widget.path ?? '');
  }

  @override
  Widget build(BuildContext context) {
    final pixelRatio = MediaQuery.devicePixelRatioOf(context);

    int? cachedSize;

    if (widget.size != null && widget.size!.isFinite) {
      cachedSize = nearestPowerOfTwo((widget.size! * pixelRatio).floor());
    }

    if (widget.path == '' || widget.path == null) {
      return widget.hint == null
          ? EmptyCoverArt(
              size: widget.size ?? double.infinity,
              index: widget.hash,
            )
          : FancyCover(
              size: widget.size ?? double.infinity,
              texts: widget.hint!,
            );
    }

    return FutureBuilder<String>(
      future: _processedPathFuture,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return EmptyCoverArt(
            size: widget.size ?? double.infinity,
            index: widget.hash,
          );
        }

        if (snapshot.hasError || !snapshot.hasData) {
          // If there's an error, show empty cover art
          return EmptyCoverArt(
            size: widget.size ?? double.infinity,
            index: widget.hash,
          );
        }

        final processedPath = snapshot.data!;

        // Use a standard Image.file widget for the processed path
        return Image.file(
          File(processedPath),
          width: widget.size ?? double.infinity,
          height: widget.size ?? double.infinity,
          fit: BoxFit.cover,
          cacheHeight: cachedSize,
          filterQuality: FilterQuality.high,
        );
      },
    );
  }
}
