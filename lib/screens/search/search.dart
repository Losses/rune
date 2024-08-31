import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar.dart';

class SearchPage extends StatefulWidget {
  const SearchPage({super.key});

  @override
  State<SearchPage> createState() => _SearchPageState();
}

class _SearchPageState extends State<SearchPage> {
  @override
  Widget build(BuildContext context) {
    return ScaffoldPage(
      content: Column(children: [
        const NavigationBarPlaceholder(),
        const TextBox(),
        Center(
          child: Text(
            'Hello, World!',
            style: FluentTheme.of(context).typography.title,
          ),
        )
      ]),
    );
  }
}
