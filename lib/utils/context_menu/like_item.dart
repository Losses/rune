import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../bindings/bindings.dart';
import '../../providers/library_open_state.dart';
import '../../providers/playing_item_capabilities.dart';
import '../../providers/request_capabilities.dart';

import '../api/request_failure.dart';
import '../l10n.dart';
import '../playing_item.dart';

/// The track [file] as the item its library keeps a like for.
PlayingItem likeableTrack(MediaFile file) => PlayingItem.fromRequest(
  PlayingItemRefLibraryTrack(libraryId: file.libraryId, track: file.track),
);

/// What the library of [item] declares about [select] for the item; available
/// where it declared nothing.
RequestCapability _itemCapability(
  BuildContext context,
  PlayingItem item,
  RequestCapability Function(PlayingItemCapabilities capabilities) select,
) =>
    Provider.of<PlayingItemCapabilitiesProvider?>(
      context,
      listen: false,
    )?.capabilityFor(item, select) ??
    const RequestCapabilityAvailable();

/// Whether a menu offers Like for the track [item], which it does where the
/// heart of the playback bar would work: the library open is not a read-only
/// replica, and both the item's library and the open one set likes.
bool trackLikeOffered(BuildContext context, PlayingItem item) =>
    !context.read<LibraryOpenStateProvider>().isReadOnlyReplica &&
    _itemCapability(context, item, (p) => p.setLikedRequest)
        is RequestCapabilityAvailable &&
    capabilityForRequest(context, (p) => p.setLikedRequest, listen: false)
        is RequestCapabilityAvailable;

/// Whether the track [item] is liked, as [fetch] answers. A library that does
/// not answer about likes, or cannot say yet, has no like to show.
Future<bool> readTrackLiked(
  BuildContext context,
  PlayingItem item,
  Future<bool> Function(PlayingItem item) fetch,
) async {
  if (_itemCapability(context, item, (p) => p.getLikedRequest)
      is! RequestCapabilityAvailable) {
    return false;
  }
  try {
    return await fetch(item);
  } on RequestFailure catch (error) {
    if (error.needsServiceSignIn ||
        error.kind == RequestErrorKind.notFound ||
        error.kind == RequestErrorKind.unavailable) {
      return false;
    }
    rethrow;
  }
}

/// The Like entry of a menu: the heart of the playback bar, filled when
/// [liked]. [onPressed] sets the opposite.
MenuFlyoutItem likeMenuItem(
  BuildContext context, {
  required bool liked,
  required VoidCallback onPressed,
}) => MenuFlyoutItem(
  leading: Icon(Symbols.favorite, fill: liked ? 1 : 0),
  text: Text(S.of(context).liked),
  onPressed: onPressed,
);

/// The Like entry as a button of the menu of a band or dock screen.
CommandBarButton likeBandButton(
  BuildContext context, {
  required bool liked,
  required VoidCallback onPressed,
}) => CommandBarButton(
  icon: Tooltip(
    message: S.of(context).liked,
    child: Icon(Symbols.favorite, fill: liked ? 1 : 0),
  ),
  onPressed: onPressed,
);
