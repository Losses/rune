//! Canonical identities for source catalog parents and set memberships.
use uuid::Uuid;

#[derive(Clone, Copy, Debug)]
pub enum ParentKind {
    Artist,
    Genre,
    Album,
}

pub fn parent(kind: ParentKind, name: &str) -> Uuid {
    let scope = match kind {
        ParentKind::Artist => "https://rune.audio/catalog/artists",
        ParentKind::Genre => "https://rune.audio/catalog/genres",
        ParentKind::Album => "https://rune.audio/catalog/albums",
    };
    Uuid::new_v5(
        &Uuid::new_v5(&Uuid::NAMESPACE_URL, scope.as_bytes()),
        name.as_bytes(),
    )
}

pub fn artwork(digest: &str) -> Uuid {
    let namespace = Uuid::new_v5(&Uuid::NAMESPACE_URL, b"https://rune.audio/catalog/artwork");
    Uuid::new_v5(&namespace, digest.as_bytes())
}

fn provenance_bytes(
    algorithm: Option<&str>,
    configuration: Option<&str>,
    format: Option<i32>,
) -> Vec<u8> {
    let mut value = Vec::new();
    for part in [algorithm, configuration] {
        match part {
            Some(part) => {
                value.push(1);
                value.extend_from_slice(&(part.len() as u64).to_be_bytes());
                value.extend_from_slice(part.as_bytes());
            }
            None => value.push(0),
        }
    }
    match format {
        Some(format) => {
            value.push(1);
            value.extend_from_slice(&format.to_be_bytes());
        }
        None => value.push(0),
    }
    value
}
pub fn fingerprint(
    file: Uuid,
    algorithm: Option<&str>,
    configuration: Option<&str>,
    format: Option<i32>,
) -> Uuid {
    let mut value = b"fingerprint:".to_vec();
    value.extend_from_slice(&provenance_bytes(algorithm, configuration, format));
    Uuid::new_v5(&file, &value)
}
pub fn similarity(
    first: Uuid,
    second: Uuid,
    algorithm: Option<&str>,
    configuration: Option<&str>,
    format: Option<i32>,
) -> Uuid {
    let mut value = b"similarity:".to_vec();
    value.extend_from_slice(second.as_bytes());
    value.extend_from_slice(&provenance_bytes(algorithm, configuration, format));
    Uuid::new_v5(&first, &value)
}

pub fn membership(file: Uuid, kind: ParentKind, parent: Uuid) -> Uuid {
    let tag: &[u8] = match kind {
        ParentKind::Artist => b"file-artist:",
        ParentKind::Genre => b"file-genre:",
        ParentKind::Album => b"file-album:",
    };
    let mut name = Vec::with_capacity(tag.len() + 16);
    name.extend_from_slice(tag);
    name.extend_from_slice(parent.as_bytes());
    Uuid::new_v5(&file, &name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_and_exact_stored_names_define_parent_identity() {
        let rock = parent(ParentKind::Genre, "Rock");
        assert_eq!(rock, parent(ParentKind::Genre, "Rock"));
        assert_ne!(rock, parent(ParentKind::Genre, "rock"));
        assert_ne!(rock, parent(ParentKind::Artist, "Rock"));
        let file = Uuid::new_v4();
        assert_ne!(
            membership(file, ParentKind::Artist, rock),
            membership(file, ParentKind::Genre, rock)
        );
    }
}
