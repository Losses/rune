import 'dart:io';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:path_provider/path_provider.dart';
import 'package:file_selector/file_selector.dart' show XTypeGroup;

import '../../../utils/l10n.dart';
import '../../api/request_failure.dart';
import '../../../utils/dialogs/unavailable_dialog_on_band.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/responsive_dialog_actions.dart';
import '../../../screens/settings_library/widgets/progress_button.dart';
import '../../../bindings/bindings.dart';

import '../mix/utils/select_input_controller.dart';
import '../mix/widgets/select_buttons_section.dart';

import 'utils/parse_size.dart';
import 'utils/render_cover_wall.dart';
import 'constants/size_items.dart';
import 'constants/frame_item.dart';
import 'constants/background_item.dart';
import '../file_dialog/pick_files.dart';

class ExportCoverWallDialog extends StatefulWidget {
  final List<(String, String)> queries;
  final String title;
  final void Function(void) $close;

  const ExportCoverWallDialog({
    super.key,
    required this.queries,
    required this.title,
    required this.$close,
  });

  @override
  ExportCoverWallDialogState createState() => ExportCoverWallDialogState();
}

class ExportCoverWallDialogState extends State<ExportCoverWallDialog> {
  bool _exportPending = false;
  bool isLoading = false;
  double progress = 0;
  String? failure;

  Playlist? playlist;

  final SelectInputController ratioController = SelectInputController('16 9');
  final SelectInputController backgroundController = SelectInputController(
    'dark',
  );
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
              if (failure != null) Text(failure!),
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
              isLoading
                  ? ProgressButton(
                      onPressed: null,
                      title: S.of(context).save,
                      progress: progress,
                      filled: true,
                    )
                  : FilledButton(
                      onPressed: onConfirmPressed,
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

  void onProgress(double x) {
    setState(() {
      progress = x;
    });
  }

  Future<void> onConfirmPressed() async {
    if (_exportPending) return;
    _exportPending = true;
    final strings = S.of(context);

    String? errorMessage;
    try {
      final documents = await getApplicationDocumentsDirectory();
      if (!mounted) return;
      final path = await pickSaveFile(
        context,
        suggestedName: '${widget.title}.png',
        initialDirectory: documents.path,
        acceptedTypeGroups: const [
          XTypeGroup(label: 'images', extensions: <String>['png']),
        ],
      );
      if (path == null || !mounted) return;

      setState(() {
        isLoading = true;
        failure = null;
      });
      final image = await renderCoverWall(
        widget.queries,
        parseSize(ratioController.selectedValue ?? '16 9'),
        backgroundController.selectedValue == 'light'
            ? Colors.white
            : Colors.black,
        frameController.selectedValue == 'enable',
        backgroundController.selectedValue == 'light'
            ? Colors.black
            : Colors.white,
        onProgress,
      );

      if (image == null) {
        errorMessage = strings.noCoverArtToExport;
        return;
      }
      try {
        final png = await image.toByteData(format: ui.ImageByteFormat.png);
        await File(path).writeAsBytes(png!.buffer.asUint8List());
      } finally {
        image.dispose();
      }
      if (mounted) widget.$close(null);
    } catch (error) {
      errorMessage = requestErrorText(strings, error);
    } finally {
      _exportPending = false;
      if (mounted) {
        setState(() {
          failure = errorMessage;
          isLoading = false;
        });
      }
    }
  }
}
