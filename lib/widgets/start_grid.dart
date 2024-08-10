import 'package:fluent_ui/fluent_ui.dart';

class StartGrid extends StatelessWidget {
  final double cellSize;
  final double gapSize;
  final List<Widget> children;

  StartGrid({
    required this.cellSize,
    required this.gapSize,
    required this.children,
  });

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final double containerHeight = constraints.maxHeight;
        final int rows = (containerHeight / (cellSize + gapSize)).floor();
        final int columns = (children.length / rows).ceil();

        return SizedBox(
          width: columns * (cellSize + gapSize) - gapSize,
          child: Wrap(
            spacing: gapSize,
            runSpacing: gapSize,
            children: children.map((child) {
              return SizedBox(
                width: cellSize,
                height: cellSize,
                child: child,
              );
            }).toList(),
          ),
        );
      },
    );
  }
}
