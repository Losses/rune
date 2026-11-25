import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/api/play_previous.dart';

import '../rune_clickable.dart';

class PreviousButton extends StatelessWidget {
  final bool disabled;
  final List<Shadow>? shadows;

  const PreviousButton({
    super.key,
    required this.disabled,
    required this.shadows,
  });

  @override
  Widget build(BuildContext context) {
    return RuneClickable(
      onPressed: disabled ? null : playPrevious,
      child: Icon(Symbols.skip_previous, shadows: shadows),
    );
  }
}
