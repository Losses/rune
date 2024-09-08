import 'package:fluent_ui/fluent_ui.dart';

List<String> completeToCompact(Iterable<String> complete, TreeViewItem root) {
  Set<String> completeSet = Set.from(complete);
  Set<String> compact = {};

  bool traverse(TreeViewItem node) {
    if (node.children.isEmpty) {
      return completeSet.contains(node.value);
    }

    bool allChildrenSelected = true;
    for (var child in node.children) {
      if (!traverse(child)) {
        allChildrenSelected = false;
      }
    }

    if (allChildrenSelected) {
      compact.add(node.value);
      return true;
    } else {
      for (var child in node.children) {
        if (completeSet.contains(child.value)) {
          compact.add(child.value);
        }
      }
      return false;
    }
  }

  traverse(root);
  return compact.toList();
}

List<String> compactToComplete(Iterable<String> compact, TreeViewItem root) {
  Set<String> compactSet = Set.from(compact);
  List<String> complete = [];

  void traverse(TreeViewItem node) {
    if (compactSet.contains(node.value)) {
      complete.add(node.value);
      for (var child in node.children) {
        traverse(child);
      }
    } else {
      for (var child in node.children) {
        traverse(child);
      }
      if (node.children.isEmpty) {
        complete.add(node.value);
      }
    }
  }

  traverse(root);
  return complete;
}
