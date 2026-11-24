//! Manual album groups and artist identity redirects for the owning library.

use crate::entities::music_metadata::album_groups as orm_album_groups;
use crate::entities::music_metadata::artists as orm_artists;
use crate::entities::music_metadata::synced_album_groups as orm_synced_album_groups;
use crate::entities::music_metadata_sync_edits as orm_sync_edits;
use sea_orm::ExprTrait;
use sea_orm::sea_query::{Alias, Expr, Func, OnConflict, Order, Query, SelectStatement};
use std::collections::BTreeSet;

use anyhow::Result;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    FromQueryResult, TransactionTrait,
};
use uuid::Uuid;

use crate::{
    error::ExpectedError,
    sync::{
        SeaOrmSyncStore,
        model::{
            Collection, MetadataEdit, ObjectKey, author_source_on, metadata_anchor_id,
            metadata_edit_id,
        },
    },
};

use super::{column, like_pattern, nocase, query_models, scalar};
use crate::entities::{
    media_file_artists,
    music_metadata::{
        album_field_values, albums, artist_redirects, audio, credits, current_album_titles, discs,
        edition_matches, local_overlays, release_identity,
    },
};

pub const PAGE_SIZE: usize = 50;

/// A selectable edition in the album manager.
#[derive(Debug, FromQueryResult)]
pub struct ReleaseEntry {
    pub id: String,
    pub title: String,
    pub version: Option<String>,
    pub catalog_number: Option<String>,
    pub release_date: Option<String>,
    pub release_type: Option<String>,
    pub directory: String,
    pub group_id: Option<String>,
    pub group_name: Option<String>,
    pub needs_review: bool,
    pub available: bool,
}

/// A selectable canonical artist in the artist manager.
#[derive(Debug, FromQueryResult)]
pub struct ArtistEntry {
    pub id: String,
    pub name: String,
    pub musicbrainz_id: Option<String>,
}

/// A named group owned by this catalog.
#[derive(Debug, FromQueryResult)]
pub struct GroupEntry {
    pub id: String,
    pub name: String,
}

/// Remove a group assignment, create a group, or join an existing group.
#[derive(Debug)]
pub enum AlbumGroupEdit {
    Remove,
    Create(String),
    Join(Uuid),
}

fn pattern(query: &str) -> String {
    let query = query
        .trim()
        .replace('!', "!!")
        .replace('%', "!%")
        .replace('_', "!_");
    format!("%{query}%")
}

/// Return one page of editions. Group members remain editable when their files are absent.
pub async fn releases(
    db: &DatabaseConnection,
    query: &str,
    group: Option<Uuid>,
    offset: u32,
) -> Result<Vec<ReleaseEntry>> {
    super::ensure_tracks_current(db).await?;
    use albums::Column as A;
    use current_album_titles::Column as T;
    use orm_album_groups::Column as G;
    let filter = pattern(query);
    let matching = Condition::any()
        .add(column(T::Title).like(like_pattern(&filter)))
        .add(column(A::DirectoryPath).like(like_pattern(&filter)))
        .add(column(T::Version).like(like_pattern(&filter)))
        .add(column(T::CatalogNumber).like(like_pattern(&filter)));
    let mut query = Query::select();

    // The group-specific plan can use the membership index directly.
    query
        .column(A::Id.as_column_ref())
        .columns(
            [
                T::Title,
                T::Version,
                T::CatalogNumber,
                T::ReleaseDate,
                T::ReleaseType,
            ]
            .map(|c| c.as_column_ref()),
        )
        .expr_as(column(A::DirectoryPath), "directory")
        .column(A::GroupId.as_column_ref())
        .expr_as(column(G::Name), "group_name")
        .column(A::NeedsReview.as_column_ref())
        .expr_as(column(T::TrackCount).gt(0), "available")
        .from(albums::Entity)
        .left_join(
            orm_album_groups::Entity,
            column(G::Id).eq(column(A::GroupId)),
        )
        .inner_join(
            current_album_titles::Entity,
            column(T::Id).eq(column(A::Id)),
        )
        .cond_where(matching);

    if let Some(group) = group {
        query.and_where(A::GroupId.eq(group.to_string()));
    }
    query
        .order_by_expr(nocase(column(T::Title)), Order::Asc)
        .order_by_columns([A::DirectoryPath, A::Id].map(|c| (c.as_column_ref(), Order::Asc)))
        .limit(PAGE_SIZE as u64 + 1)
        .offset(u64::from(offset));
    query_models(db, &query).await
}

/// Return one page of existing groups matching a name.
pub async fn album_groups(
    db: &DatabaseConnection,
    query: &str,
    offset: u32,
) -> Result<Vec<GroupEntry>> {
    use orm_album_groups::Column as G;
    let filter = pattern(query);
    let mut query = Query::select();
    query
        .columns([G::Id, G::Name])
        .from(orm_album_groups::Entity)
        .and_where(column(G::Name).like(like_pattern(&filter)))
        .order_by_expr(nocase(column(G::Name)), Order::Asc)
        .order_by(G::Id, Order::Asc)
        .limit(PAGE_SIZE as u64 + 1)
        .offset(u64::from(offset));
    query_models(db, &query).await
}

/// Return canonical artists, matching either the main name or any merged name.
pub async fn artists(
    db: &DatabaseConnection,
    query: &str,
    offset: u32,
) -> Result<Vec<ArtistEntry>> {
    use artist_redirects::Column as R;
    use orm_artists::Column as A;
    let filter = pattern(query);
    let alias = Alias::new("merged_artist");
    let former = Query::select()
        .column(R::SourceUuid.as_column_ref())
        .from(artist_redirects::Entity)
        .join_as(
            sea_orm::JoinType::InnerJoin,
            orm_artists::Entity,
            alias.clone(),
            Expr::col((alias.clone(), A::Id)).eq(column(R::SourceUuid)),
        )
        .and_where(column(R::TargetUuid).eq(column(A::Id)))
        .and_where(Expr::col((alias, A::Name)).like(like_pattern(&filter)))
        .to_owned();
    let credited = Query::select()
        .column(credits::Column::FileId.as_column_ref())
        .from(credits::Entity)
        .and_where(column(credits::Column::ArtistId).eq(column(A::Id)))
        .and_where(column(credits::Column::CreditedAs).like(like_pattern(&filter)))
        .to_owned();

    // Search canonical names and current credit spellings without reviving merged artists.
    let mut query = Query::select();
    query
        .columns([A::Id, A::Name, A::MusicbrainzId])
        .from(orm_artists::Entity)
        .and_where(visible_artist())
        .and_where(
            column(A::Name)
                .like(like_pattern(&filter))
                .or(Expr::exists(former))
                .or(Expr::exists(credited)),
        )
        .order_by_expr(nocase(column(A::Name)), Order::Asc)
        .order_by(A::Id, Order::Asc)
        .limit(PAGE_SIZE as u64 + 1)
        .offset(u64::from(offset));
    query_models(db, &query).await
}

fn visible_artist() -> Expr {
    use artist_redirects::Column as R;
    use credits::Column as C;
    use media_file_artists::Column as L;
    use orm_artists::Column as A;
    let redirect = Query::select()
        .column(R::SourceUuid.as_column_ref())
        .from(artist_redirects::Entity)
        .and_where(column(R::SourceUuid).eq(column(A::Id)))
        .to_owned();
    let known = Alias::new("verified_artist");
    let homonym = Query::select()
        .column((known.clone(), A::Id))
        .from_as(orm_artists::Entity, known.clone())
        .and_where(Expr::col((known.clone(), A::MusicbrainzId)).is_not_null())
        .and_where(Expr::col((known, A::Name)).eq(nocase(column(A::Name))))
        .to_owned();

    // A name shared with a verified artist remains visible if any tag is still unlinked.
    let linked = Query::select()
        .column(C::FileId.as_column_ref())
        .from(credits::Entity)
        .and_where(column(C::FileId).eq(column(audio::Column::FileId)))
        .and_where(C::Scope.eq(2))
        .and_where(C::Role.eq("Primary Artist"))
        .and_where(C::ArtistId.is_not_null())
        .and_where(column(C::CreditedAs).eq(nocase(column(A::Name))))
        .to_owned();
    let unlinked = Query::select()
        .column(L::TrackUuid.as_column_ref())
        .from(media_file_artists::Entity)
        .left_join(
            audio::Entity,
            column(audio::Column::Id).eq(column(L::TrackUuid)),
        )
        .and_where(column(L::ArtistUuid).eq(column(A::Id)))
        .and_where(Expr::not_exists(linked))
        .to_owned();
    Expr::not_exists(redirect).and(
        A::MusicbrainzId
            .is_not_null()
            .or(Expr::not_exists(homonym))
            .or(Expr::exists(unlinked)),
    )
}

/// Update multiple memberships in the same transaction.
pub async fn edit_album_group(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    releases: &[Uuid],
    edit: AlbumGroupEdit,
) -> Result<Option<Uuid>> {
    let releases = selection(releases)?;
    let tx = db.begin().await?;
    store.lock(&tx).await?;

    // Confirm all selected releases still exist before changing any membership.
    for &id in &releases {
        require(
            &tx,
            Query::select()
                .column(albums::Column::Id)
                .from(albums::Entity)
                .and_where(albums::Column::Id.eq(id.to_string())),
            "release",
        )
        .await?;
    }
    let group = match edit {
        AlbumGroupEdit::Remove => None,
        AlbumGroupEdit::Join(id) => {
            require(
                &tx,
                Query::select()
                    .column(orm_album_groups::Column::Id)
                    .from(orm_album_groups::Entity)
                    .and_where(orm_album_groups::Column::Id.eq(id.to_string())),
                "album group",
            )
            .await?;
            Some(id)
        }
        AlbumGroupEdit::Create(name) => {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 256 {
                return Err(invalid("Album group name must contain 1 to 256 characters"));
            }
            let group = Uuid::new_v4();
            tx.execute(
                Query::insert()
                    .into_table(orm_album_groups::Entity)
                    .columns([orm_album_groups::Column::Id, orm_album_groups::Column::Name])
                    .values_panic([Expr::val(group.to_string()), Expr::val(name.to_owned())]),
            )
            .await?;
            Some(group)
        }
    };

    let label = if let Some(group) = group {
        let row = tx
            .query_one(
                Query::select()
                    .column(orm_album_groups::Column::Name)
                    .from(orm_album_groups::Entity)
                    .and_where(
                        Expr::col(orm_album_groups::Column::Id).eq(Expr::val(group.to_string())),
                    ),
            )
            .await?
            .ok_or(ExpectedError::NotFound {
                what: "album group",
            })?;
        Some(row.try_get::<String>("", "name")?)
    } else {
        None
    };

    // Publish matching evidence separately from the edition's assignment.
    for release in releases {
        let (edition, generation) = edition_for_edit(&tx, release).await?;
        publish_edition_anchors(&tx, store, edition, release).await?;
        author_edit(
            &tx,
            store,
            MetadataEdit {
                kind: "album".to_owned(),
                owner: edition,
                edition: None,
                target: group,
                label: label.clone(),
                generation,
            },
        )
        .await?;
    }
    cleanup_groups(&tx).await?;
    tx.commit().await?;
    Ok(group)
}

async fn cleanup_groups(tx: &DatabaseTransaction) -> Result<()> {
    use albums::Column as A;
    use orm_album_groups::Column as G;
    use orm_sync_edits::Column as E;
    let proposed = Query::select()
        .column(album_field_values::Column::AlbumId)
        .from(album_field_values::Entity)
        .and_where(column(album_field_values::Column::AlbumId).eq(column(A::Id)))
        .to_owned();
    let overridden = Query::select()
        .column(local_overlays::Column::AlbumId)
        .from(local_overlays::Entity)
        .and_where(column(local_overlays::Column::AlbumId).eq(column(A::Id)))
        .to_owned();
    let media = Query::select()
        .column(discs::Column::AlbumId)
        .from(discs::Entity)
        .and_where(column(discs::Column::AlbumId).eq(column(A::Id)))
        .to_owned();

    // Keep editions with either authored state or a current disc.
    tx.execute(
        Query::delete()
            .from_table(albums::Entity)
            .and_where(A::GroupId.is_null())
            .and_where(Expr::not_exists(proposed))
            .and_where(Expr::not_exists(overridden))
            .and_where(Expr::not_exists(media)),
    )
    .await?;

    // A synced edit may still name a group whose edition has not arrived.
    let members = Query::select()
        .column(A::Id.as_column_ref())
        .from(albums::Entity)
        .and_where(column(A::GroupId).eq(column(G::Id)))
        .to_owned();
    let edits = Query::select()
        .column(E::Id.as_column_ref())
        .from(orm_sync_edits::Entity)
        .and_where(E::Kind.eq("album"))
        .and_where(column(E::TargetUuid).eq(column(G::Id)))
        .to_owned();
    tx.execute(
        Query::delete()
            .from_table(orm_album_groups::Entity)
            .and_where(Expr::not_exists(members))
            .and_where(Expr::not_exists(edits)),
    )
    .await?;
    Ok(())
}

async fn edition_for_edit(db: &DatabaseTransaction, release: Uuid) -> Result<(Uuid, i64)> {
    let row = db
        .query_one(
            Query::select()
                .expr_as(
                    Expr::col(orm_synced_album_groups::Column::OwnerUuid),
                    Alias::new("id"),
                )
                .column(orm_synced_album_groups::Column::Generation)
                .from(orm_synced_album_groups::Entity)
                .and_where(
                    Expr::col(orm_synced_album_groups::Column::AlbumId)
                        .eq(Expr::val(release.to_string())),
                )
                .order_by(orm_synced_album_groups::Column::Generation, Order::Desc)
                .order_by(orm_synced_album_groups::Column::OwnerUuid, Order::Asc)
                .limit(1),
        )
        .await?;
    if let Some(row) = row {
        let generation = row.try_get::<i64>("", "generation")?;
        return Ok((
            super::uuid(&row, "id")?,
            generation
                .checked_add(1)
                .ok_or_else(|| invalid("Edition edit generation overflow"))?,
        ));
    }
    Ok((portable_edition(db, release).await?, 1))
}

pub(in crate::actions) async fn portable_edition(
    db: &impl ConnectionTrait,
    release: Uuid,
) -> Result<Uuid> {
    use edition_matches::Column as M;
    use release_identity::Column as I;
    let key = release.to_string();
    let other = Alias::new("other_edition");
    let duplicate = Query::select()
        .column((other.clone(), I::AlbumId))
        .from_as(release_identity::Entity, other.clone())
        .and_where(Expr::col((other.clone(), I::EditionUuid)).eq(column(I::EditionUuid)))
        .and_where(Expr::col((other, I::AlbumId)).ne(&key))
        .to_owned();
    let unique = Query::select()
        .column(I::EditionUuid.as_column_ref())
        .from(release_identity::Entity)
        .and_where(I::AlbumId.eq(&key))
        .and_where(Expr::not_exists(duplicate))
        .to_owned();

    // Prefer the exact identity and fall back to a matching anchor or the local UUID.
    let anchored = Query::select()
        .column(M::EditionUuid)
        .from(edition_matches::Entity)
        .and_where(M::AlbumId.eq(&key))
        .and_where(M::EditionUuid.ne(&key))
        .order_by(M::EditionUuid, Order::Asc)
        .limit(1)
        .to_owned();
    let row = db
        .query_one(Query::select().expr_as(
            Func::coalesce([scalar(unique), scalar(anchored), Expr::val(key)]),
            "id",
        ))
        .await?
        .ok_or(ExpectedError::NotFound { what: "release" })?;
    super::uuid(&row, "id")
}

pub(in crate::actions) async fn publish_edition_anchors(
    tx: &DatabaseTransaction,
    store: &SeaOrmSyncStore,
    edition: Uuid,
    release: Uuid,
) -> Result<()> {
    // Only current shared tracks can establish identity on another replica.
    use orm_sync_edits::Column as E;
    let anchored = Query::select()
        .column(E::Id.as_column_ref())
        .from(orm_sync_edits::Entity)
        .and_where(E::Kind.eq("anchor"))
        .and_where(E::OwnerUuid.eq(edition.to_string()))
        .and_where(column(E::EditionUuid).eq(column(audio::Column::Id)))
        .to_owned();
    let rows = tx
        .query_all(
            Query::select()
                .column(audio::Column::Id.as_column_ref())
                .from(discs::Entity)
                .inner_join(
                    audio::Entity,
                    column(audio::Column::DiscId).eq(column(discs::Column::Id)),
                )
                .and_where(discs::Column::AlbumId.eq(release.to_string()))
                .and_where(Expr::not_exists(anchored)),
        )
        .await?;
    for row in rows {
        let track = super::uuid(&row, "id")?;
        author_edit(
            tx,
            store,
            MetadataEdit {
                kind: "anchor".to_owned(),
                owner: edition,
                edition: Some(track),
                target: None,
                label: None,
                generation: 0,
            },
        )
        .await?;
    }
    Ok(())
}

async fn author_edit(
    db: &DatabaseTransaction,
    store: &SeaOrmSyncStore,
    edit: MetadataEdit,
) -> Result<()> {
    // Derive the identifier with the same edit fields used by sync validation.
    let MetadataEdit {
        kind,
        owner,
        edition,
        target,
        label,
        generation,
    } = edit;
    let id = if kind == "anchor" {
        metadata_anchor_id(
            owner,
            edition.ok_or_else(|| invalid("Missing shared track"))?,
        )
    } else {
        metadata_edit_id(&kind, owner)
    };
    db.execute(
        Query::insert()
            .into_table(orm_sync_edits::Entity)
            .columns([
                orm_sync_edits::Column::SourceUuid,
                orm_sync_edits::Column::Kind,
                orm_sync_edits::Column::OwnerUuid,
                orm_sync_edits::Column::EditionUuid,
                orm_sync_edits::Column::TargetUuid,
                orm_sync_edits::Column::Label,
                orm_sync_edits::Column::Generation,
            ])
            .values_panic([
                Expr::val(id.to_string()),
                Expr::val(kind),
                Expr::val(owner.to_string()),
                Expr::val(edition.map(|id| id.to_string())),
                Expr::val(target.map(|id| id.to_string())),
                Expr::val(label),
                Expr::val(generation),
            ])
            .on_conflict(
                OnConflict::column(orm_sync_edits::Column::SourceUuid)
                    .update_columns([
                        orm_sync_edits::Column::TargetUuid,
                        orm_sync_edits::Column::Label,
                        orm_sync_edits::Column::Generation,
                    ])
                    .to_owned(),
            ),
    )
    .await?;
    author_source_on(
        db,
        store,
        ObjectKey {
            collection: Collection::MetadataEdits,
            id,
        },
    )
    .await
}

/// Merge canonical artists while preserving redirect references on replicas.
pub async fn merge_artists(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    target: Uuid,
    sources: &[Uuid],
) -> Result<()> {
    let sources = selection(sources)?;
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let target = canonical_artist(&tx, target).await?;
    let row = tx
        .query_one(
            Query::select()
                .column(orm_artists::Column::Name)
                .from(orm_artists::Entity)
                .and_where(Expr::col(orm_artists::Column::Id).eq(Expr::val(target.to_string()))),
        )
        .await?
        .ok_or(ExpectedError::NotFound { what: "artist" })?;
    let name: String = row.try_get("", "name")?;

    // Resolve old selections before updating any mapping. Repeating a merge is harmless.
    let mut canonical = BTreeSet::new();
    for source in sources {
        let source = canonical_artist(&tx, source).await?;
        if source != target {
            canonical.insert(source);
        }
    }
    for source in canonical {
        author_edit(
            &tx,
            store,
            MetadataEdit {
                kind: "artist".to_owned(),
                owner: source,
                edition: None,
                target: Some(target),
                label: Some(name.clone()),
                generation: 0,
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Assign a pending set of tracks to a known edition before the next scan.
pub async fn confirm_release(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    pending: Uuid,
    edition: Uuid,
) -> Result<()> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    super::release_group::confirm(&tx, pending, edition).await?;
    tx.commit().await?;
    Ok(())
}

async fn require(
    db: &impl ConnectionTrait,
    query: &SelectStatement,
    what: &'static str,
) -> Result<()> {
    if db.query_one(query).await?.is_none() {
        return Err(ExpectedError::NotFound { what }.into());
    }
    Ok(())
}

/// Resolve a current or merged artist UUID to its canonical identity.
pub async fn canonical_artist(db: &impl ConnectionTrait, id: Uuid) -> Result<Uuid> {
    use artist_redirects::Column as R;
    use orm_artists::Column as A;
    let row = db
        .query_one(
            Query::select()
                .expr_as(Func::coalesce([column(R::TargetUuid), column(A::Id)]), "id")
                .from(orm_artists::Entity)
                .left_join(
                    artist_redirects::Entity,
                    column(R::SourceUuid).eq(column(A::Id)),
                )
                .and_where(A::Id.eq(id.to_string())),
        )
        .await?
        .ok_or(ExpectedError::NotFound { what: "artist" })?;
    super::uuid(&row, "id")
}

fn selection(ids: &[Uuid]) -> Result<BTreeSet<Uuid>> {
    if ids.is_empty() || ids.len() > 512 {
        return Err(invalid("Select between 1 and 512 entries"));
    }
    Ok(ids.iter().copied().collect())
}

fn invalid(detail: &str) -> anyhow::Error {
    ExpectedError::InvalidInput {
        detail: detail.to_owned(),
    }
    .into()
}
