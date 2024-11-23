import 'dart:io';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:file_selector/file_selector.dart';

import '../../../utils/l10n.dart';
import '../../../utils/dialogs/unavailable_dialog_on_band.dart';
import '../../../utils/dialogs/export_cover_wall/utils/render_cover_wall.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/responsive_dialog_actions.dart';
import '../../../messages/playlist.pb.dart';
import '../../../messages/collection.pb.dart';

class ExportCoverWallDialog extends StatefulWidget {
  final CollectionType type;
  final int id;
  final void Function(void) $close;

  const ExportCoverWallDialog({
    super.key,
    required this.type,
    required this.id,
    required this.$close,
  });

  @override
  ExportCoverWallDialogState createState() => ExportCoverWallDialogState();
}

class ExportCoverWallDialogState extends State<ExportCoverWallDialog> {
  bool isLoading = false;

  Playlist? playlist;

  @override
  void initState() {
    super.initState();
  }

  @override
  dispose() {
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return UnavailableDialogOnBand(
      $close: widget.$close,
      child: NoShortcuts(
        ContentDialog(
          title: Column(
            children: [],
          ),
          constraints: BoxConstraints(
            maxWidth: 800,
            maxHeight: 600,
          ),
          actions: [
            ResponsiveDialogActions(
              FilledButton(
                onPressed: isLoading
                    ? null
                    : () async {
                        setState(() {
                          isLoading = true;
                        });

                        final image =
                            await renderCoverWall(widget.type, widget.id);

                        final FileSaveLocation? result = await getSaveLocation(
                          suggestedName: 'cover_wall.png',
                          acceptedTypeGroups: const [
                            XTypeGroup(
                              label: 'images',
                              extensions: <String>['png'],
                            )
                          ],
                        );

                        if (result == null) return;

                        final pngBytes = await image.toByteData(
                          format: ui.ImageByteFormat.png,
                        );

                        File(result.path).writeAsBytesSync(
                          pngBytes!.buffer.asInt8List(),
                        );

                        setState(() {
                          isLoading = false;
                        });

                        widget.$close(null);
                      },
                child: Text(S.of(context).save),
              ),
              Button(
                onPressed: isLoading ? null : () => widget.$close(null),
                child: Text(S.of(context).cancel),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
