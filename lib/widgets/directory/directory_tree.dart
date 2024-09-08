import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/widgets/directory/utils/convertion.dart';

import '../../../messages/directory.pb.dart';

class DirectoryTree extends StatefulWidget {
  final Future<void> Function(Iterable<String>) onSelectionChanged;

  const DirectoryTree({super.key, required this.onSelectionChanged});

  @override
  State<DirectoryTree> createState() => _DirectoryTreeState();
}

class _DirectoryTreeState extends State<DirectoryTree> {
  Future<TreeViewItem> directoryTree = fetchAndConvertDirectoryTree();

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
            return TreeView(
              selectionMode: TreeViewSelectionMode.multiple,
              scrollPrimary: true,
              shrinkWrap: true,
              items: snapshot.data!.children,
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
                final Iterable<String> items =
                    selectedItems.map((i) => i.value.toString());

                return widget.onSelectionChanged(
                    completeToCompact(items, snapshot.data!));
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
