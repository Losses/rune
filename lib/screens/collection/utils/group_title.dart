import 'package:flutter/widgets.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/start_screen/utils/group.dart';

/// The title of a group a library fills itself with what was played lately,
/// however the library's service titles it. Keep it the same as
/// `RECENTLY_PLAYED_GROUP` in `native/hub/src/apple_music_translate.rs`.
const recentlyPlayedGroupKey = 'recently-played';

/// The title of a group a library fills itself and its service gives no
/// title. Keep it the same as `UNTITLED_GROUP` in
/// `native/hub/src/apple_music_translate.rs`.
const untitledGroupKey = 'untitled';

/// The title a group of a collection page shows. A group the library fills
/// itself under one of the keys above is named in the language of the app; the
/// group keeps its title. Any other title, an empty one included, is shown as
/// it is.
String collectionGroupTitle(BuildContext context, Group<dynamic> group) {
  if (!group.reserved) return group.groupTitle;
  return switch (group.groupTitle) {
    recentlyPlayedGroupKey => S.of(context).recentlyPlayed,
    untitledGroupKey => S.of(context).forYou,
    final title => title,
  };
}
