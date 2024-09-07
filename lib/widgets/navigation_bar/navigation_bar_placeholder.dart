import 'package:fluent_ui/fluent_ui.dart';

const navigationBarHeight = 64.0 + 40;

class NavigationBarPlaceholder extends StatelessWidget {
  const NavigationBarPlaceholder({super.key});

  @override
  Widget build(BuildContext context) {
    return const SizedBox(height: navigationBarHeight);
  }
}
