//! What the item this device plays supports of the requests answered by the library the item
//! belongs to, as the UI is told it.

use std::ptr;
use std::sync::Arc;

use anyhow::Result;
use database::library_registry::{LibraryRecord, LibraryRecordId, LibraryRegistry};
use log::warn;
use playback::player::PlayingItem;
use uuid::Uuid;

use super::{ItemEventState, ResolvedLibrary};
use crate::{
    macros::{RequestSlots, item_capability_snapshot},
    messages::{
        GetLikedRequest, GetLyricByTrackIdRequest, GetPrimaryColorByTrackIdRequest,
        PlayingItemCapabilities, SetLikedRequest,
    },
    node::NodeContext,
    request_owner::Implementation,
    rpc::{Slot, UnsupportedReason},
    service::{ServiceBinding, item_slots},
    utils::{Broadcaster, held_service_binding, load_registry},
};

/// What the UI was last told the current item supports.
#[derive(Default)]
pub(super) struct Declared {
    told: Option<PlayingItemCapabilities>,
    /// Whether what the item supports could not be told since.
    untold: bool,
}

/// Tells the UI what the item last reported supports, when that is not what it was last told.
/// Nothing is sent when that cannot be told now: the UI keeps what it heard, and a later status
/// tells it through [`send_if_untold`].
pub(super) fn send(
    node: &NodeContext,
    item_state: &mut ItemEventState,
    broadcaster: &dyn Broadcaster,
) {
    let local_library_open = item_state.request_ui_library.is_some();
    send_with(node, item_state, broadcaster, local_library_open);
}

/// Like [`send`], only when what the item supports could not be told the last time.
pub(super) fn send_if_untold(
    node: &NodeContext,
    item_state: &mut ItemEventState,
    broadcaster: &dyn Broadcaster,
) {
    if item_state.capabilities.untold {
        send(node, item_state, broadcaster);
    }
}

/// Like [`send`], with `local_library_open` saying whether the UI has a local library open.
fn send_with(
    node: &NodeContext,
    item_state: &mut ItemEventState,
    broadcaster: &dyn Broadcaster,
    local_library_open: bool,
) {
    let Some(slots) = playing_item_slots(
        node,
        item_state.last_item.as_ref(),
        item_state.library.as_ref(),
        local_library_open,
    ) else {
        item_state.capabilities.untold = true;
        return;
    };
    item_state.capabilities.untold = false;
    let capabilities = item_capability_snapshot(item_state.request.clone(), &slots);
    if item_state.capabilities.told.as_ref() != Some(&capabilities) {
        broadcaster.broadcast(&capabilities);
        item_state.capabilities.told = Some(capabilities);
    }
}

/// What the item supports may have changed without a status: a service library signed in or
/// lost its sign-in, the UI opened or closed a library, or a library record went away. The item's
/// library is looked up again when it is no longer this device's to answer for it, and the UI
/// hears what the item supports now.
pub(super) async fn changed(
    node: &NodeContext,
    item_state: &mut ItemEventState,
    broadcaster: &dyn Broadcaster,
) {
    if let Some(ResolvedLibrary {
        library_id,
        implementation: Implementation::Service(binding),
        ..
    }) = &item_state.library
        && let_go(node, *library_id, binding)
    {
        item_state.library = None;
    }
    let local_library_open = super::current_ui_library_id(node).await.is_some();
    send_with(node, item_state, broadcaster, local_library_open);
}

/// Whether the binding the item's library resolved to no longer answers for it: the library's
/// record is gone, as after a removal, or its service signs in with its binding and this device
/// let that binding go. Any other binding, as one made from its library's record, stays the
/// item's.
fn let_go(node: &NodeContext, library_id: Uuid, binding: &Arc<dyn ServiceBinding>) -> bool {
    let removed =
        load_registry(node).is_ok_and(|registry| record_of(&registry, library_id).is_err());
    let released = binding.service().signs_in_on_this_device()
        && !held_service_binding(node, library_id)
            .ok()
            .flatten()
            .is_some_and(|held| ptr::addr_eq(Arc::as_ptr(&held), Arc::as_ptr(binding)));
    removed || released
}

/// The item slots of the library `item` belongs to: those of the implementation it resolved to,
/// or of the binding held for it since, as after a sign-in, or else what its service declares
/// for the items of a library this device holds no binding for. None when the library records
/// cannot be read.
fn playing_item_slots(
    node: &NodeContext,
    item: Option<&PlayingItem>,
    library: Option<&ResolvedLibrary>,
    local_library_open: bool,
) -> Option<RequestSlots> {
    if let Some(library) = library {
        return Some(library.implementation.item_slots());
    }
    let library_id = match item {
        Some(PlayingItem::LibraryTrack { library_id, .. }) => *library_id,
        Some(_) => return Some(file_slots(local_library_open)),
        None => return Some(RequestSlots::local()),
    };
    match held_service_binding(node, library_id) {
        Ok(Some(binding)) => Some(Implementation::Service(binding).item_slots()),
        Ok(None) => unheld_item_slots(load_registry(node), library_id),
        Err(error) => {
            warn!("Could not tell what the playing item supports: {error:#}");
            None
        }
    }
}

/// The record of the library `library_id`: a service library's id is the id of its record, while
/// a local or replica library's is its catalog's, which its record names.
fn record_of(registry: &LibraryRegistry, library_id: Uuid) -> Result<&LibraryRecord> {
    registry
        .record(LibraryRecordId(library_id))
        .or_else(|_| registry.catalog(library_id))
}

/// The item slots of the library `library_id`, which this device holds no binding for, from
/// the library records as `registry` read them. None when they could not be read: the UI then
/// keeps what it heard rather than hearing that a local library answers.
fn unheld_item_slots(registry: Result<LibraryRegistry>, library_id: Uuid) -> Option<RequestSlots> {
    let registry = match registry {
        Ok(registry) => registry,
        Err(error) => {
            warn!("Could not read the library records for the playing item: {error:#}");
            return None;
        }
    };
    Some(match record_of(&registry, library_id) {
        Ok(record) => record
            .protocol
            .service()
            .map_or_else(RequestSlots::local, item_slots),
        // The library is gone from this device, as after its record was removed: nothing
        // about the item can be answered.
        Err(_) => RequestSlots::unimplemented(),
    })
}

/// What a request about an item of no library does. A file played by its path has no liked
/// state, and its lyrics and color are read through the local library open in the UI, without
/// which they cannot be read.
fn file_slots(local_library_open: bool) -> RequestSlots {
    let refused = Slot::Unsupported {
        reason: UnsupportedReason::NotImplemented,
    };
    let mut slots = RequestSlots::local();
    slots.set::<SetLikedRequest>(refused);
    slots.set::<GetLikedRequest>(refused);
    if !local_library_open {
        slots.set::<GetLyricByTrackIdRequest>(refused);
        slots.set::<GetPrimaryColorByTrackIdRequest>(refused);
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::file_slots;
    use crate::macros::item_capability_snapshot;
    use crate::messages::{PlayingItemCapabilities, RequestCapability};
    use crate::request_owner::Implementation;

    fn each(capabilities: &PlayingItemCapabilities) -> [RequestCapability; 4] {
        [
            capabilities.set_liked_request,
            capabilities.get_liked_request,
            capabilities.get_lyric_by_track_id_request,
            capabilities.get_primary_color_by_track_id_request,
        ]
    }

    #[test]
    fn a_peer_answers_about_its_items_and_a_file_has_no_likes() {
        use RequestCapability::{Available, Hidden};

        // The session resolves an item it plays to the library held here, so a peer answers
        // only for an item a peer holds: every request then goes to that peer.
        let peer = Implementation::RunePeer {
            link: None,
            reconnecting: None,
        };
        assert_eq!(
            each(&item_capability_snapshot(None, &peer.item_slots())),
            [Available; 4]
        );
        // A file played by its path has no liked state, and is read through a local library.
        assert_eq!(
            each(&item_capability_snapshot(None, &file_slots(true))),
            [Hidden, Hidden, Available, Available]
        );
        assert_eq!(
            each(&item_capability_snapshot(None, &file_slots(false))),
            [Hidden; 4]
        );
    }
}
