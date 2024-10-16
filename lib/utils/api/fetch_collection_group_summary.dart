import 'dart:async';

import 'package:rune/messages/collection.pb.dart';

Future<List<CollectionGroupSummary>> fetchCollectionGroupSummary(
  CollectionType collectionType,
) async {
  final request =
      FetchCollectionGroupSummaryRequest(collectionType: collectionType);
  request.sendSignalToRust(); // GENERATED

  return (await CollectionGroupSummaryResponse.rustSignalStream.first)
      .message
      .groups;
}
