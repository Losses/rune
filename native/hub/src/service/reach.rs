//! Whether this device is online, and which tracks it stores, for the libraries whose tracks
//! can play only while the device is online or has them stored.

use std::collections::HashSet;

use crate::messages::AudioAvailability;

/// What the platform host reported of this device's network. Online until it says otherwise.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reach {
    /// The ids of the tracks stored on this device, as the library that holds them names
    /// them; none while the device is online.
    stored: Option<HashSet<String>>,
}

impl Reach {
    /// The device is offline, and `stored` are the ids of the tracks it holds.
    pub(crate) fn offline(stored: HashSet<String>) -> Self {
        Self {
            stored: Some(stored),
        }
    }

    /// Whether the device is online.
    pub(crate) fn is_online(&self) -> bool {
        self.stored.is_none()
    }

    /// Whether a track plays now. `follows` is what its library declares (see
    /// `ServiceBinding::availability_follows_connectivity`): a library that does not follow
    /// connectivity is not affected by it. One that does plays everything while online, and
    /// offline only the tracks stored on this device, which `stored_id` names; a track
    /// with no such id is not stored.
    pub(crate) fn availability(&self, follows: bool, stored_id: Option<&str>) -> AudioAvailability {
        match &self.stored {
            Some(stored) if follows && !stored_id.is_some_and(|id| stored.contains(id)) => {
                AudioAvailability::Unavailable
            }
            _ => AudioAvailability::RemoteOnly,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_that_follows_connectivity_plays_only_what_is_stored_offline() {
        let offline = Reach::offline(HashSet::from(["a".to_owned()]));
        assert_eq!(
            offline.availability(true, Some("a")),
            AudioAvailability::RemoteOnly
        );
        assert_eq!(
            offline.availability(true, Some("b")),
            AudioAvailability::Unavailable
        );
        assert_eq!(
            offline.availability(true, None),
            AudioAvailability::Unavailable
        );
        assert_eq!(
            Reach::default().availability(true, None),
            AudioAvailability::RemoteOnly
        );
    }

    #[test]
    fn a_library_that_does_not_follow_connectivity_is_unchanged_offline() {
        let offline = Reach::offline(HashSet::new());
        assert_eq!(
            offline.availability(false, None),
            AudioAvailability::RemoteOnly
        );
        assert_eq!(
            offline.availability(false, Some("a")),
            AudioAvailability::RemoteOnly
        );
    }
}
