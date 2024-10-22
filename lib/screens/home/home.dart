import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:file_selector/file_selector.dart';

import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../providers/library_path.dart';

class HomePage extends StatefulWidget {
  const HomePage({super.key});

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  Future<void> showHistoryDialog(
      BuildContext context, LibraryPathProvider provider) async {
    showDialog(
      context: context,
      builder: (context) {
        List<String> allOpenedFiles = provider.getAllOpenedFiles();
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
                    provider.setLibraryPath(allOpenedFiles[index]);

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
  }

  @override
  Widget build(BuildContext context) {
    return Consumer<LibraryPathProvider>(builder: (context, provider, child) {
      return PageContentFrame(
        child: Center(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              if (provider.currentPath != null)
                Text('Current Path: ${provider.currentPath}'),
              if (provider.currentPath == null)
                Button(
                  onPressed: () async {
                    final result = await getDirectoryPath();

                    if (result == null) {
                      return;
                    }
                    await provider.setLibraryPath(result);
                  },
                  child: const Text("Create Library"),
                ),
              if (provider.currentPath == null)
                Button(
                  onPressed: () async {
                    await showHistoryDialog(context, provider);
                  },
                  child: const Text("Select from History"),
                ),
            ],
          ),
        ),
      );
    });
  }
}
