import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';

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

class CoverArt extends StatelessWidget {
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
  Widget build(BuildContext context) {
    final pixelRatio = MediaQuery.devicePixelRatioOf(context);

    int? cachedSize;

    if (size != null && size!.isFinite) {
      cachedSize = (size! * pixelRatio).floor();
    }

    return path == '' || path == null
        ? hint == null
            ? EmptyCoverArt(
                size: size ?? double.infinity,
                index: hash,
              )
            : FancyCover(
                size: size ?? double.infinity,
                texts: hint!,
              )
        : Image.file(
            File(path!),
            width: size ?? double.infinity,
            height: size ?? double.infinity,
            fit: BoxFit.cover,
            cacheHeight: cachedSize,
            cacheWidth: cachedSize,
          );
  }
}
