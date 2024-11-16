import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../messages/collection.pb.dart';

final List<(CollectionType, String Function(BuildContext))> searchCategories = [
  (CollectionType.Track, (context) => S.of(context).tracks),
  (CollectionType.Artist, (context) => S.of(context).artists),
  (CollectionType.Album, (context) => S.of(context).albums),
  (CollectionType.Playlist, (context) => S.of(context).playlists),
];
