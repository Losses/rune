import 'package:fluent_ui/fluent_ui.dart';

void printFocusNodes() {
  final FocusScopeNode rootScope =
      WidgetsBinding.instance.focusManager.rootScope;
  final buffer = StringBuffer();
  buffer.writeln('');
  buffer.writeln('Current Node: ${primaryFocus?.hashCode}');
  buffer.writeln('');
  _collectFocusNodeTree(rootScope, 0, buffer);
  debugPrint(buffer.toString());
}

void _collectFocusNodeTree(FocusNode node, int depth, StringBuffer buffer) {
  final indent = '  ' * depth;
  final debugLabel = node.debugLabel ?? 'No Debug Label';
  buffer.writeln('$indent- Node(${node.hashCode}): $debugLabel');

  for (final FocusNode child in node.children) {
    _collectFocusNodeTree(child, depth + 1, buffer);
  }
}
