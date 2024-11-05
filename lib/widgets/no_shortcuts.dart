import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/config/shortcuts.dart';

class NoShortcuts extends StatelessWidget {
  const NoShortcuts(this.widget, {super.key});

  final Widget widget;

  @override
  Widget build(BuildContext context) {
    return Shortcuts(
      shortcuts: noShortcuts,
      child: widget,
    );
  }
}
