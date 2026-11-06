import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../screens/home/widgets/welcome.dart';
import '../../screens/home/widgets/select_library.dart';
import '../../providers/library_path.dart';
import '../../widgets/keep_clear.dart';

class HomePage extends StatelessWidget {
  const HomePage({super.key});

  @override
  Widget build(BuildContext context) {
    final libraryPath = Provider.of<LibraryPathProvider>(context);

    return KeepClear(
      child: libraryPath.libraryHistory.isEmpty
          ? const WelcomePage()
          : const SelectLibraryPage(),
    );
  }
}
