import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';

import '../../../../bindings/bindings.dart';

import '../file_dialog_icons.dart';
import '../file_dialog_names.dart';
import '../file_dialog_thumbnails.dart';

/// An item's icon: the platform's thumbnail when it draws one and the view
/// is large enough, the type icon otherwise and until the thumbnail comes.
class FileDialogItemIcon extends StatefulWidget {
  const FileDialogItemIcon({
    super.key,
    required this.entry,
    required this.names,
    required this.size,
    required this.thumbnails,
  });

  final FileDialogEntry entry;
  final FileDialogNames names;
  final double size;

  /// Null where thumbnails are not shown.
  final FileDialogThumbnails? thumbnails;

  @override
  State<FileDialogItemIcon> createState() => _FileDialogItemIconState();
}

class _FileDialogItemIconState extends State<FileDialogItemIcon> {
  ui.Image? _image;
  (FileDialogLocation, int?, int)? _requested;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _request();
  }

  @override
  void didUpdateWidget(covariant FileDialogItemIcon oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.entry != widget.entry ||
        oldWidget.size != widget.size ||
        oldWidget.thumbnails != widget.thumbnails) {
      _request();
    }
  }

  void _request() {
    final thumbnails = widget.thumbnails;
    final entry = widget.entry;
    if (thumbnails == null) {
      _release();
      return;
    }
    final size = FileDialogThumbnails.requestSize(
      widget.size,
      MediaQuery.devicePixelRatioOf(context),
    );
    final key = (entry.location, entry.modified?.toInt(), size);
    if (key == _requested) return;
    _release();
    _requested = key;
    thumbnails.load(key.$1, key.$2, key.$3).then((image) {
      if (!mounted || _requested != key) {
        image?.dispose();
        return;
      }
      setState(() => _image = image);
    });
  }

  void _release() {
    final requested = _requested;
    if (requested != null && _image == null) {
      widget.thumbnails?.drop(requested.$1, requested.$2, requested.$3);
    }
    _requested = null;
    _image?.dispose();
    _image = null;
  }

  @override
  void dispose() {
    _release();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final image = _image;
    final size = widget.size;
    if (image == null) {
      return FileDialogAssetIcon(
        widget.names.entryIcon(widget.entry, size),
        size: size,
      );
    }
    return SizedBox.square(
      dimension: size,
      child: Center(
        child: RawImage(
          image: image,
          fit: BoxFit.contain,
          filterQuality: FilterQuality.medium,
        ),
      ),
    );
  }
}
