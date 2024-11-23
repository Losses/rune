import 'dart:io';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:file_selector/file_selector.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../../utils/l10n.dart';
import '../../../utils/dialogs/unavailable_dialog_on_band.dart';
import '../../../utils/dialogs/export_cover_wall/utils/render_cover_wall.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/responsive_dialog_actions.dart';
import '../../../messages/playlist.pb.dart';
import '../../../messages/collection.pb.dart';
import '../mix/utils/select_input_controller.dart';
import '../mix/widgets/select_buttons_section.dart';
import '../mix/widgets/select_input_section.dart';

List<SelectItem> sizeItems(BuildContext context) => [
      SelectItem(
        value: '16 9',
        title: '¹⁶⁄₉',
        icon: Symbols.crop_16_9,
      ),
      SelectItem(
        value: '3 2',
        title: '³⁄₂',
        icon: Symbols.crop_3_2,
      ),
      SelectItem(
        value: '7 5',
        title: '⁷⁄₅',
        icon: Symbols.crop_7_5,
      ),
      SelectItem(
        value: '5 4',
        title: '⁵⁄₄',
        icon: Symbols.crop_5_4,
      ),
      SelectItem(
        value: '1 1',
        title: '¹⁄₁',
        icon: Symbols.crop_square,
      ),
      SelectItem(
        value: '9 16',
        title: '⁹⁄₁₆',
        icon: Symbols.crop_9_16,
      ),
    ];

List<SelectItem> backgroundItem(BuildContext context) => [
      SelectItem(
        value: 'dark',
        title: S.of(context).dark,
        icon: Symbols.dark_mode,
      ),
      SelectItem(
        value: 'light',
        title: S.of(context).light,
        icon: Symbols.light_mode,
      ),
    ];

List<SelectItem> frameItem(BuildContext context) => <SelectItem>[
      SelectItem(
        value: 'enable',
        title: S.of(context).enable,
        icon: Symbols.iframe,
      ),
      SelectItem(
        value: 'disable',
        title: S.of(context).disable,
        icon: Symbols.iframe_off,
      ),
    ];

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

  final SelectInputController ratioController = SelectInputController('16 9');
  final SelectInputController backgroundController =
      SelectInputController('dark');
  final SelectInputController frameController = SelectInputController('enable');

  @override
  void initState() {
    super.initState();
  }

  @override
  dispose() {
    super.dispose();
    ratioController.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return UnavailableDialogOnBand(
      $close: widget.$close,
      child: NoShortcuts(
        ContentDialog(
          title: Text(S.of(context).exportCoverWall),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              SelectButtonsSection(
                controller: ratioController,
                title: S.of(context).ratio,
                items: sizeItems,
                defaultValue: '16 9',
                rows: 2,
              ),
              SelectButtonsSection(
                controller: backgroundController,
                title: S.of(context).background,
                items: backgroundItem,
                defaultValue: 'dark',
              ),
              SelectButtonsSection(
                controller: frameController,
                title: S.of(context).frame,
                items: frameItem,
                defaultValue: 'enable',
              ),
            ],
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
