import 'package:fluent_ui/fluent_ui.dart';

class TracksPage extends StatefulWidget {
  const TracksPage({super.key});

  @override
  State<TracksPage> createState() => _TracksPageState();
}

class _TracksPageState extends State<TracksPage> {
  @override
  Widget build(BuildContext context) {
    return ScaffoldPage(
      header: const PageHeader(
        title: Text('Hello World with Fluent UI'),
      ),
      content: Center(
        child: Text(
          'Hello, World!',
          style: FluentTheme.of(context).typography.title,
        ),
      ),
    );
  }
}
