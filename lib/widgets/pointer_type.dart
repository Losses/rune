import 'dart:ui';

import 'package:fluent_ui/fluent_ui.dart';

class PointerTypeBuilder extends StatefulWidget {
  final Widget Function(PointerDeviceKind kind) builder;
  const PointerTypeBuilder({
    super.key,
    required this.builder,
  });

  @override
  PointerTypeBuilderState createState() => PointerTypeBuilderState();
}

class PointerTypeBuilderState extends State<PointerTypeBuilder> {
  PointerDeviceKind kind = PointerDeviceKind.unknown;

  @override
  Widget build(BuildContext context) {
    return Listener(
      onPointerHover: (event) {
        if (kind != event.kind) {
          setState(() {
            kind = event.kind;
          });
        }
      },
      child: widget.builder(kind),
    );
  }
}
