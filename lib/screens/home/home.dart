import 'package:fluent_ui/fluent_ui.dart';

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
              MediaLibraryPath(
                path: '/home/losses/Music/party',
              ).sendSignalToRust(); // GENERATED
            },
            child: const Text("Send Media Library Path to Rust"),
          ),
        ]),
      ),
    );
  }
}
