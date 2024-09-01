import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../widgets/navigation_bar.dart';
import '../../messages/library_manage.pb.dart';
import '../../providers/library_path.dart';

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
                onPressed: () async {
                  await closeLibrary(context);

                  if (!context.mounted) return;

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

Future<void> closeLibrary(BuildContext context) async {
  final library = Provider.of<LibraryPathProvider>(context, listen: false);

  final path = library.currentPath;
  CloseLibraryRequest(path: path).sendSignalToRust();

  while (true) {
    final rustSignal = await CloseLibraryResponse.rustSignalStream.first;

    if (rustSignal.message.path == path) {
      return;
    }
  }
}
