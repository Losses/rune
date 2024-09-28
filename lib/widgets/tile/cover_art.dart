import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/tile/fancy_cover.dart';

class EmptyCoverArt extends StatelessWidget {
  final double? size;

  const EmptyCoverArt({super.key, this.size});

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      color: Colors.green,
      child: Icon(
        Symbols.album,
        size: size == null ? null : size! * 0.8,
      ),
    );
  }
}

class CoverArt extends StatelessWidget {
  final String? path;
  final (String, String, String)? hint;
  final double? size;

  const CoverArt({
    super.key,
    required this.path,
    this.size,
    this.hint,
  });

  @override
  Widget build(BuildContext context) {
    return path == '' || path == null
        ? hint == null
            ? EmptyCoverArt(
                size: size ?? double.infinity,
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
          );
  }
}
