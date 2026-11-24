import 'package:flutter/widgets.dart';

import '../../../bindings/bindings.dart';
import '../../../providers/request_capabilities.dart';

bool userGenerated(CollectionType collectionType) {
  return collectionType == CollectionType.playlist ||
      collectionType == CollectionType.mix;
}

/// Whether the user can create an item of this type in the open library. A
/// library that keeps no mixes of its own does not let the user create them,
/// so its pages do not suggest it. Set [listen] when calling it while building.
bool canCreateCollection(
  BuildContext context,
  CollectionType collectionType, {
  bool listen = false,
}) {
  if (collectionType != CollectionType.mix) return true;
  return requestAvailable(
    context,
    (provider) => provider.createMixRequest,
    listen: listen,
  );
}

/// Whether the empty page of a collection type suggests how to fill it. A page
/// of collections the user cannot create has nothing to suggest. Call it while
/// building: the page follows the open library's capabilities.
bool emptyPageShowsDetail(BuildContext context, CollectionType collectionType) {
  return !userGenerated(collectionType) ||
      canCreateCollection(context, collectionType, listen: true);
}

/// Whether the open library can create a playlist from an M3U8 file.
bool canImportM3u8Playlist(BuildContext context) {
  return requestAvailable(
    context,
    (provider) => provider.createM3u8PlaylistRequest,
  );
}
