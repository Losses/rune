import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';

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
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return GestureDetector(
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
    );
  }
}
