import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:file_picker/file_picker.dart';

import '../../messages/connection.pb.dart';

class HomePage extends StatefulWidget {
  const HomePage({super.key});

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  @override
  Widget build(BuildContext context) {
    return ScaffoldPage(
      header: const PageHeader(
        title: Text('Hello World with Fluent UI'),
      ),
      content: Center(
        child: Column(mainAxisAlignment: MainAxisAlignment.center, children: [
          Button(
            onPressed: () async {
              // Allow manually define the library path by running with
              // `flutter run --dart-define=LIBRARY_PATH=/path/to/library`
              String libraryPath = const String.fromEnvironment('LIBRARY_PATH',
                  defaultValue: "");
              if (libraryPath.isEmpty) {
                final result = await FilePicker.platform.getDirectoryPath();
                if (result == null) {
                  return;
                }
                libraryPath = result;
              }
              MediaLibraryPath(
                path: libraryPath,
              ).sendSignalToRust(); // GENERATED
            },
            child: const Text("Send Media Library Path to Rust"),
          ),
        ]),
      ),
    );
  }
}
