import 'package:fluent_ui/fluent_ui.dart';

class Scaled extends StatefulWidget {
  final Widget child;
  final double scale;

  const Scaled({super.key, required this.scale, required this.child});

  @override
  State<Scaled> createState() => _ScaledState();
}

class _ScaledState extends State<Scaled> {
  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        // Use a GlobalKey to measure the size of the child widget
        final GlobalKey childKey = GlobalKey();

        // Create a widget to measure the child
        final measureChild = Container(
          key: childKey,
          child: widget.child,
        );

        return FutureBuilder(
          future: WidgetsBinding.instance.endOfFrame,
          builder: (context, snapshot) {
            if (snapshot.connectionState == ConnectionState.done) {
              // Get the size of the child widget
              final RenderBox renderBox =
                  childKey.currentContext!.findRenderObject() as RenderBox;
              final size = renderBox.size;

              // Create a container with double the size of the child
              return SizedBox(
                width: size.width * widget.scale,
                height: size.height * widget.scale,
                child: Center(
                    child: Transform.scale(scale: widget.scale, child: measureChild)),
              );
            } else {
              // Return the child widget while waiting for the frame to end
              return measureChild;
            }
          },
        );
      },
    );
  }
}
