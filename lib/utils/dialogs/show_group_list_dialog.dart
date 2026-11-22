import '../api/request_failure.dart';
import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/router/navigation.dart';
import '../../widgets/start_screen/utils/group.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';
import '../../screens/collection/utils/collection_data_provider.dart';
import '../../utils/l10n.dart';

/// Loads groups for a jump and lets the list's Retry button resume that jump.
mixin CollectionGroupNavigation<T extends StatefulWidget> on State<T> {
  ({String title, Future<Object?> summary})? _pendingJump;

  /// Scrolls the layout to an index in the current collection.
  void scrollToGroupIndex(CollectionDataProvider data, int index);

  /// Finds the selected group, fetching additional pages as needed.
  Future<void> scrollToGroup(String title) async {
    if (!mounted) return;
    final data = context.read<CollectionDataProvider>();
    final jump = (title: title, summary: data.summary);
    _pendingJump = jump;
    while (mounted && _pendingJump == jump && data.summary == jump.summary) {
      final index = data.items.indexWhere((group) => group.groupTitle == title);
      if (index >= 0) {
        _pendingJump = null;
        scrollToGroupIndex(data, index);
        return;
      }
      if (data.isLastPage) {
        _pendingJump = null;
        return;
      }
      await data.fetchData();
      if (data.pageError != null) return;
    }
  }

  /// Resumes a pending jump for this collection or retries ordinary pagination.
  Future<void> retryGroupOrPage(Future<void> Function() fetchPage) {
    final jump = _pendingJump;
    if (jump != null &&
        jump.summary == context.read<CollectionDataProvider>().summary) {
      return scrollToGroup(jump.title);
    }
    return fetchPage();
  }
}

void showGroupListDialog(
  BuildContext context,
  Future<void> Function(String) scrollToGroup,
) async {
  final data = Provider.of<CollectionDataProvider>(context, listen: false);
  await $showModal<void>(
    context,
    (context, $close) => FutureBuilder<List<Group<InternalCollection>>>(
      future: data.summary,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return Container();
        } else if (snapshot.hasError) {
          return Center(child: Text(requestErrorText(S.of(context), snapshot.error!)));
        } else {
          return ContentDialog(
            constraints: const BoxConstraints(maxWidth: 320),
            content: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              crossAxisAlignment: CrossAxisAlignment.end,
              mainAxisSize: MainAxisSize.min,
              children: [
                Wrap(
                  spacing: 4,
                  runSpacing: 4,
                  children: snapshot.data!
                      .map(
                        (x) => ConstrainedBox(
                          constraints: const BoxConstraints(maxWidth: 40),
                          child: AspectRatio(
                            aspectRatio: 1,
                            child: Button(
                              child: Text(x.groupTitle),
                              onPressed: () {
                                $close(0);
                                scrollToGroup(x.groupTitle);
                              },
                            ),
                          ),
                        ),
                      )
                      .toList(),
                ),
                const SizedBox(height: 24),
                Button(
                  child: Text(S.of(context).cancel),
                  onPressed: () => $close(0),
                ),
              ],
            ),
          );
        }
      },
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
