import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/request_capabilities.dart';
import '../../utils/query_list.dart';
import '../../utils/router/query_tracks_parameter.dart';
import '../query_tracks/query_tracks.dart';

import 'artist_page.dart';

/// The page of the artist [arguments] name: the artist page where the open
/// library builds one, the artist's tracks otherwise.
///
/// The page follows the library's capabilities, so one built before the first snapshot
/// arrives is replaced when the snapshot says the library builds an artist page.
Widget artistDetailPage(BuildContext context, QueryTracksParameter arguments) {
  return Builder(
    builder: (context) {
      if (canOpenArtistPage(context, listen: true)) {
        return ArtistPage(artistId: arguments.id, name: arguments.title);
      }
      return QueryTracksPage(
        queries: QueryList([("lib::artist", arguments.id.toString())]),
        title: arguments.title,
        mode: null,
      );
    },
  );
}
