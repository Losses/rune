import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/api/play_next.dart';

import '../rune_icon_button.dart';

class NextButton extends StatelessWidget {
  final bool disabled;
  final List<Shadow>? shadows;

  const NextButton({
    super.key,
    required this.disabled,
    required this.shadows,
  });

  @override
  Widget build(BuildContext context) {
    return RuneIconButton(
      onPressed: disabled ? null : playNext,
      icon: Icon(
        Symbols.skip_next,
        shadows: shadows,
      ),
    );
  }
}
