import 'package:fluent_ui/fluent_ui.dart';

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
        child: Text(
          'Hello, World!',
          style: FluentTheme.of(context).typography.title,
        ),
      ),
    );
  }
}
