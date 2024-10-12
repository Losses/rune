import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/widgets/ax_pressure.dart';

import '../../widgets/hover_opacity.dart';

class LinkTurntile extends StatefulWidget {
  final String title;
  final String path;
  final IconData? icon;

  const LinkTurntile({
    super.key,
    required this.title,
    required this.path,
    this.icon,
  });

  @override
  State<LinkTurntile> createState() => _LinkTurntileState();
}

class _LinkTurntileState extends State<LinkTurntile> {
  final FocusNode _focusNode = FocusNode();

  void onPressed() {
    context.push(widget.path);
  }

  @override
  void dispose() {
    _focusNode.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return AxPressure(
      child: GestureDetector(
        onTap: onPressed,
        child: HoverOpacity(
          child: FocusableActionDetector(
            focusNode: _focusNode,
            child: Text(
              widget.title,
              textAlign: TextAlign.start,
              style: theme.typography.title?.apply(fontWeightDelta: -100),
            ),
          ),
        ),
      ),
    );
  }
}
