import 'package:fluent_ui/fluent_ui.dart';
import 'package:file_selector/file_selector.dart';

import '../../utils/file_storage_service.dart';
import '../../messages/connection.pb.dart';

class HomePage extends StatefulWidget {
  const HomePage({super.key});

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  final FileStorageService _fileStorageService = FileStorageService();

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
                final result = await getDirectoryPath();
                if (result == null) {
                  return;
                }
                libraryPath = result;
              }

              // Store the selected library path
              await _fileStorageService.storeFilePath(libraryPath);

              // Send the signal to Rust
              MediaLibraryPath(
                path: libraryPath,
              ).sendSignalToRust(); // GENERATED
            },
            child: const Text("Select Library"),
          ),
          Button(
            onPressed: () async {
              // Show a dialog with the list of previously opened libraries
              showDialog(
                context: context,
                builder: (context) {
                  List<String> allOpenedFiles =
                      _fileStorageService.getAllOpenedFiles();
                  return ContentDialog(
                    title: const Text('Select from History'),
                    content: SizedBox(
                      width: double.maxFinite,
                      child: ListView.builder(
                        shrinkWrap: true,
                        itemCount: allOpenedFiles.length,
                        itemBuilder: (context, index) {
                          return ListTile(
                            title: Text(allOpenedFiles[index]),
                            onPressed: () {
                              // Send the signal to Rust with the selected path
                              MediaLibraryPath(
                                path: allOpenedFiles[index],
                              ).sendSignalToRust(); // GENERATED
                              Navigator.pop(context); // Close the dialog
                            },
                          );
                        },
                      ),
                    ),
                    actions: [
                      Button(
                        child: const Text('Close'),
                        onPressed: () => Navigator.pop(context),
                      ),
                    ],
                  );
                },
              );
            },
            child: const Text("Select from History"),
          ),
        ]),
      ),
    );
  }
}
