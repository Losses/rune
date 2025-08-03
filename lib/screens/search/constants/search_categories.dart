import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../bindings/bindings.dart';

final List<(CollectionType, String Function(BuildContext))> searchCategories = [
  (CollectionType.track, (context) => S.of(context).tracks),
  (CollectionType.artist, (context) => S.of(context).artists),
  (CollectionType.album, (context) => S.of(context).albums),
  (CollectionType.playlist, (context) => S.of(context).playlists),
];
