import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/playback_controller.dart';
import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';
import '../../messages/directory.pb.dart';

class SettingsTestPage extends StatefulWidget {
  const SettingsTestPage({super.key});

  @override
  State<SettingsTestPage> createState() => _SettingsTestPageState();
}

class _SettingsTestPageState extends State<SettingsTestPage> {
  Future<void> onSelectionChanged(Iterable<TreeViewItem> selectedItems) async {
    debugPrint('${selectedItems.map((i) => i.value)}');
  }

  @override
  Widget build(BuildContext context) {
    return Column(children: [
      const NavigationBarPlaceholder(),
      Expanded(child: DirectoryTree(onSelectionChanged: onSelectionChanged)),
      const PlaybackPlaceholder()
    ]);
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

class DirectoryTree extends StatefulWidget {
  final Future<void> Function(Iterable<TreeViewItem>) onSelectionChanged;

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
              onSelectionChanged: widget.onSelectionChanged,
            );
          }
        });
  }
}
