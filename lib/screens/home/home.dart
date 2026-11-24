import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../screens/home/widgets/welcome.dart';
import '../../screens/home/widgets/select_library.dart';
import '../../providers/library_registry.dart';

class HomePage extends StatelessWidget {
  const HomePage({super.key});

  @override
  Widget build(BuildContext context) {
    final registry = Provider.of<LibraryRegistryProvider>(context);

    return SafeArea(
      bottom: false,
      child: registry.records.isEmpty
          ? const WelcomePage()
          : const SelectLibraryPage(),
    );
  }
}
