import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/router/navigation.dart';
import '../../widgets/start_screen/utils/group.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';
import '../../screens/collection/utils/collection_data_provider.dart';
import '../../utils/l10n.dart';

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
          return Center(child: Text('Error: ${snapshot.error}'));
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
