import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/api/play_next.dart';

class NextButton extends StatelessWidget {
  final bool disabled;

  const NextButton({required this.disabled, super.key});

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: disabled ? null : playNext,
      icon: const Icon(Symbols.skip_next),
    );
  }
}
