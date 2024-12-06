import 'package:fluent_ui/fluent_ui.dart';

import '../settings_card.dart';
import '../../../../utils/dialogs/export_cover_wall/show_export_cover_wall_dialog.dart';

class LibraryCoverWallpaperSettings extends StatelessWidget {
  const LibraryCoverWallpaperSettings({super.key});

  @override
  Widget build(BuildContext context) {
    return SettingsCard(
      title: "Library Cover Wallpaper",
      description:
          "Render a cover wall that includes all tracks. Please be aware that this feature may cause the software to crash due to insufficient available memory.",
      content: FilledButton(
        onPressed: () => showExportCoverWallDialog(
          context,
          [("lib::directory.deep", "/")],
          "Rune",
        ),
        child: Text("Getting Started"),
      ),
    );
  }
}
