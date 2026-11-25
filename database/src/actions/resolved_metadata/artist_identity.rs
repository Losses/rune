//! Match an explicit MusicBrainz artist only to an unambiguous primary credit.

use anyhow::Result;
use metadata::model::MetadataBase;
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, Set, sea_query::OnConflict};
use uuid::Uuid;

use crate::actions::utils::generate_group_name;
use crate::entities::music_metadata::artists;

use super::{external_uuid, lifecycle::BrowseKind};

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
    artists::Entity::insert(artists::ActiveModel {
        id: Set(id.clone()),
        name: Set(credit.credited_as.clone()),
        group: Set(generate_group_name(&credit.credited_as)),
        musicbrainz_id: Set(Some(mbid.to_string())),
    })
    .on_conflict(
        OnConflict::column(artists::Column::Id)
            .update_column(artists::Column::MusicbrainzId)
            .action_and_where(artists::Column::MusicbrainzId.is_null())
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    super::lifecycle::allocate_handle(db, BrowseKind::Artist, &id).await?;
    Ok(Some(id))
}
