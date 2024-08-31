import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/providers/library_path.dart';
import 'package:provider/provider.dart';

import '../../widgets/navigation_bar.dart';

class SettingsPage extends StatefulWidget {
  const SettingsPage({super.key});

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  @override
  Widget build(BuildContext context) {
    return ScaffoldPage(
      content: Column(children: [
        const NavigationBarPlaceholder(),
        Center(
          child: Column(
            children: [
              Button(
                onPressed: () {
                  Provider.of<LibraryPathProvider>(context, listen: false)
                      .clearAllOpenedFiles();
                },
                child: const Text("Factory Reset"),
              )
            ],
          ),
        )
      ]),
    );
  }
}
