import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/directory/utils/convertion.dart';
import '../../bindings/bindings.dart';

class DirectoryTreeController extends ChangeNotifier {
  Set<String>? _selectedValue;

  DirectoryTreeController([selectedValue])
      : _selectedValue = selectedValue ?? {};

  Set<String>? get value => _selectedValue;

  set value(Set<String>? value) {
    if (_selectedValue != value) {
      _selectedValue = value;
      notifyListeners();
    }
  }
}

class DirectoryTree extends StatefulWidget {
  final Future<void> Function(Iterable<String>)? onSelectionChanged;
  final DirectoryTreeController? controller;

  const DirectoryTree({
    super.key,
    this.onSelectionChanged,
    this.controller,
  });

  @override
  State<DirectoryTree> createState() => _DirectoryTreeState();
}

class _DirectoryTreeState extends State<DirectoryTree> {
  late final DirectoryTreeController _controller;
  Future<TreeViewItem> directoryTree = fetchAndConvertDirectoryTree();

  @override
  void initState() {
    super.initState();
    _controller = widget.controller ?? DirectoryTreeController({});
  }

  @override
  void dispose() {
    if (widget.controller == null) {
      _controller.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<TreeViewItem>(
        future: directoryTree,
        builder: (context, snapshot) {
          if (snapshot.connectionState == ConnectionState.waiting) {
            return Container();
          } else if (snapshot.hasError) {
            return Center(child: Text('Error: ${snapshot.error}'));
          } else if (!snapshot.hasData) {
            return const Center(child: Text('No data available'));
          } else {
            final root = TreeViewItem(
              content: const Text('/'),
              value: '/',
              expanded: true,
              children: snapshot.data!.children,
            );

            final value = _controller.value;

            if (value != null) {
              updateTreeViewSelection(root, value);
            }

            return TreeView(
              selectionMode: TreeViewSelectionMode.multiple,
              scrollPrimary: true,
              shrinkWrap: true,
              items: [root],
              onItemInvoked: (item, reason) async {
                if (reason == TreeViewItemInvokeReason.pressed) {
                  setState(() {
                    if (item.children.isNotEmpty) {
                      item.expanded = !item.expanded;
                    } else {
                      item.selected = !(item.selected ?? false);
                    }
                  });
                }
              },
              onSelectionChanged: (selectedItems) async {
                _controller.value = Set.from(
                    selectedItems.map((x) => x.value as String).toList());

                if (widget.onSelectionChanged != null) {
                  final Iterable<String> items =
                      selectedItems.map((i) => i.value.toString());

                  widget.onSelectionChanged!(
                    completeToCompact(items, snapshot.data!),
                  );
                }
              },
            );
          }
        });
  }
}

TreeViewItem convertDirectoryTree(DirectoryTreeResponse tree) {
  return TreeViewItem(
    content: Text(tree.name),
    value: tree.path,
    expanded: false,
    children: tree.children.isNotEmpty
        ? tree.children.map(convertDirectoryTree).toList()
        : const [],
  );
}

Future<DirectoryTreeResponse> fetchDirectoryTree() async {
  FetchDirectoryTreeRequest().sendSignalToRust();

  final rustSignal = await FetchDirectoryTreeResponse.rustSignalStream.first;
  final root = rustSignal.message.root;

  return root;
}

Future<TreeViewItem> fetchAndConvertDirectoryTree() async {
  final root = await fetchDirectoryTree();

  return convertDirectoryTree(root);
}

void updateTreeViewSelection(TreeViewItem item, Set<String> selectedValues) {
  item.selected = selectedValues.contains(item.value);

  for (var child in item.children) {
    updateTreeViewSelection(child, selectedValues);
  }
}
