//! Canonical identities for source catalog parents and set memberships.
use uuid::Uuid;

#[derive(Clone, Copy, Debug)]
pub enum ParentKind {
    Artist,
    Genre,
    Album,
}

// Fixed namespaces for catalog identities. The values are frozen: every
// artist, genre, album and artwork id in existing libraries is derived from
// them, so changing one re-keys the whole catalog.
const ARTIST_NAMESPACE: Uuid = Uuid::from_u128(0x8abe01e7_2bff_5bf7_83e5_5384e894b7ea);
const GENRE_NAMESPACE: Uuid = Uuid::from_u128(0xbf8bc345_35d3_52de_9eda_98cb40b6e66a);
const ALBUM_NAMESPACE: Uuid = Uuid::from_u128(0xcb7db76b_a1e9_5b23_a0af_fffed19fad14);
const ARTWORK_NAMESPACE: Uuid = Uuid::from_u128(0x415eb90e_9ca3_525d_816b_f00dd6adf865);

pub fn parent(kind: ParentKind, name: &str) -> Uuid {
    let namespace = match kind {
        ParentKind::Artist => ARTIST_NAMESPACE,
        ParentKind::Genre => GENRE_NAMESPACE,
        ParentKind::Album => ALBUM_NAMESPACE,
    };
    Uuid::new_v5(&namespace, name.as_bytes())
}

pub fn artwork(digest: &str) -> Uuid {
    Uuid::new_v5(&ARTWORK_NAMESPACE, digest.as_bytes())
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

    #[test]
    fn catalog_identities_are_frozen() {
        let cases = [
            (
                parent(ParentKind::Genre, "Rock"),
                0xfe10e0ca_5afa_5ebc_acb9_d52d188d0cff,
            ),
            (
                parent(ParentKind::Artist, "Rock"),
                0xebcada7c_e383_5af0_8789_e94ede911d4b,
            ),
            (
                parent(ParentKind::Album, "Rock"),
                0x92151a79_832a_55b5_ab8b_7324612f4d4f,
            ),
            (artwork("abc"), 0x9afadddf_9519_56ce_8b69_7006a0c1c6d9),
        ];
        for (actual, expected) in cases {
            assert_eq!(actual, Uuid::from_u128(expected));
        }
    }
}
