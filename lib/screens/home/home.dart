import 'package:fluent_ui/fluent_ui.dart';
import 'package:file_selector/file_selector.dart';
import 'package:provider/provider.dart';

import '../../providers/library_path.dart';

class HomePage extends StatefulWidget {
  const HomePage({super.key});

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  @override
  Widget build(BuildContext context) {
    return Consumer<LibraryPathProvider>(builder: (context, provider, child) {
      return ScaffoldPage(
          header: const PageHeader(
            title: Text('Hello World with Fluent UI'),
          ),
          content: Center(
            child:
                Column(mainAxisAlignment: MainAxisAlignment.center, children: [
              if (provider.currentPath != null)
                Text('Current Path: ${provider.currentPath}'),
              if (provider.currentPath == null)
                Button(
                  onPressed: () async {
                    String libraryPath = const String.fromEnvironment(
                        'LIBRARY_PATH',
                        defaultValue: "");
                    if (libraryPath.isEmpty) {
                      final result = await getDirectoryPath();
                      if (result == null) {
                        return;
                      }
                      libraryPath = result;
                    }

                    if (!context.mounted) return;
                    // Store the selected library path and update the provider
                    await Provider.of<LibraryPathProvider>(context,
                            listen: false)
                        .setLibraryPath(libraryPath);
                  },
                  child: const Text("Select Library"),
                ),
              if (provider.currentPath == null)
                Button(
                  onPressed: () async {
                    showDialog(
                      context: context,
                      builder: (context) {
                        List<String> allOpenedFiles =
                            Provider.of<LibraryPathProvider>(context,
                                    listen: false)
                                .getAllOpenedFiles();
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
                                    // Update the current path in the provider
                                    Provider.of<LibraryPathProvider>(context,
                                            listen: false)
                                        .setLibraryPath(allOpenedFiles[index]);

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
          ));
    });
  }
}
