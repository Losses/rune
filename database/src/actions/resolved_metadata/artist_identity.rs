//! Match an explicit MusicBrainz artist only to an unambiguous primary credit.

use anyhow::Result;
use metadata::model::MetadataBase;
use sea_orm::{ConnectionTrait, DatabaseTransaction};
use uuid::Uuid;

use super::{external_uuid, statement};

const NAMESPACE: Uuid = Uuid::from_u128(0xa512c2e6_5010_5d7a_8971_dcdf1881bb4d);
const VARIOUS_ARTISTS: Uuid = Uuid::from_u128(0x89ad4ac3_39f7_470e_963a_56509c546377);

pub(super) async fn primary(
    db: &DatabaseTransaction,
    base: &MetadataBase,
    identifier: &str,
) -> Result<Option<String>> {
    let Some(mbid) = external_uuid(base, identifier).filter(|id| *id != VARIOUS_ARTISTS) else {
        return Ok(None);
    };
    let mut primary = base
        .credits
        .iter()
        .flatten()
        .filter(|credit| credit.role == "Primary Artist");
    let Some(credit) = primary.next().filter(|_| primary.next().is_none()) else {
        return Ok(None);
    };

    // The MBID identifies the artist. The spelling remains on each contributing credit.
    let id = Uuid::new_v5(&NAMESPACE, mbid.as_bytes()).to_string();
    db.execute_raw(statement(
        "INSERT INTO music_metadata_artists (id, name, musicbrainz_id) VALUES (?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET musicbrainz_id = excluded.musicbrainz_id
         WHERE musicbrainz_id IS NULL",
        [
            id.clone().into(),
            credit.credited_as.clone().into(),
            mbid.to_string().into(),
        ],
    ))
    .await?;
    Ok(Some(id))
}
