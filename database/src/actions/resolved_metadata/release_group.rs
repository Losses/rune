//! Resolve release membership before assigning a persistent catalog UUID.

use crate::entities::music_metadata::albums as orm_albums;
use crate::entities::music_metadata::audio as orm_audio;
use crate::entities::music_metadata::dirty_directories as orm_dirty_directories;
use crate::entities::music_metadata::track_release_keys as orm_track_release_keys;
use sea_orm::ExprTrait;
use sea_orm::sea_query::{Expr, OnConflict, Order, Query, SelectStatement};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::LazyLock,
};

use anyhow::Result;
use metadata::{
    model::{AlbumSource, MetadataBase, MetadataSourceKind},
    resolve::LocalMetadata,
};
use regex::Regex;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, TransactionTrait,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::entities::{
    music_metadata::{discs, release_identity},
    track_album_sources,
};

use super::column;
use super::lifecycle::BrowseKind;

const TITLE: &str = "title";
const ARTIST: &str = "artist";
const VERSION: &str = "version";
const RELEASE: &str = "release";
const CATALOG: &str = "catalog";
const MEDIUM_PREFIX: &str = "medium.";

static FINAL_DISC_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(.*?)[\s,._\-–(\[]+(?:cd|disc|disque|disk)[ _.-]*([0-9]+)[)\]]?$")
        .expect("valid final disc marker")
});
static INITIAL_DISC_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:cd|disc|disque|disk)[ _.-]*([0-9]+)[\s:._\-–]+(.+)$")
        .expect("valid initial disc marker")
});

#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ReleaseKeys(BTreeMap<String, String>);

impl ReleaseKeys {
    pub(super) fn from_track(track: &LocalMetadata) -> Self {
        let mut keys = Self::default();
        let album = &track.album;
        let disc = &track.disc;

        // Folder fallbacks describe the source directory, not an independent match key.
        if has_tag(&album.base, TITLE) {
            keys.add(TITLE, Some(album_title(track)));
        }
        if has_tag(&album.base, VERSION) {
            keys.add(VERSION, album.base.version.as_deref());
        }
        keys.add(CATALOG, album.catalog_number.as_deref());
        keys.add(RELEASE, release_id(&album.base).as_deref());

        // Only explicit album artists participate in matching across tracks.
        let names: BTreeSet<_> = album
            .base
            .credits
            .iter()
            .flatten()
            .filter(|credit| credit.role == "Primary Artist")
            .map(|credit| normalize(&credit.credited_as))
            .filter(|name| !name.is_empty() && name != "various artists")
            .collect();
        if !names.is_empty() {
            keys.0.insert(
                ARTIST.to_owned(),
                names.into_iter().collect::<Vec<_>>().join("\0"),
            );
        }
        if let Some(path) = &disc.directory_path {
            // Physical media paths stay comparable after moving the album directory.
            let AlbumSource::Local { directory_path } = &album.source else {
                return keys;
            };
            let path = Path::new(path)
                .strip_prefix(directory_path)
                .ok()
                .and_then(Path::to_str)
                .unwrap_or(path);
            keys.0.insert(
                format!("{MEDIUM_PREFIX}{}", disc.disc_number),
                path.to_owned(),
            );
        }
        keys
    }

    fn add(&mut self, key: &str, value: Option<&str>) {
        if let Some(value) = value.map(normalize).filter(|value| !value.is_empty()) {
            self.0.insert(key.to_owned(), value);
        }
    }
}

fn has_tag(base: &MetadataBase, field: &str) -> bool {
    base.provenance
        .as_ref()
        .and_then(|fields| fields.get(field))
        .is_some_and(|sources| {
            sources
                .iter()
                .any(|source| source.kind == MetadataSourceKind::AudioTag)
        })
}

pub(super) fn album_title(track: &LocalMetadata) -> &str {
    let title = &track.album.title;
    if has_tag(&track.album.base, TITLE)
        && (track.disc.directory_path.is_some() || track.audio.disc_number.is_some())
    {
        title_without_disc(title, track.disc.disc_number)
    } else {
        title
    }
}

pub(super) fn title_priority(track: &LocalMetadata) -> i32 {
    i32::from(!has_tag(&track.album.base, TITLE))
}

pub(super) fn version_priority(track: &LocalMetadata) -> i32 {
    i32::from(!has_tag(&track.album.base, VERSION))
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn release_id(base: &MetadataBase) -> Option<String> {
    let ids: BTreeSet<_> = base
        .identifiers
        .as_ref()?
        .get("musicbrainz:release")?
        .iter()
        .filter_map(|id| Uuid::parse_str(id.trim()).ok())
        .collect();
    if ids.len() == 1 {
        ids.first().map(Uuid::to_string)
    } else {
        None
    }
}

fn title_without_disc(title: &str, number: u32) -> &str {
    // Strip only a marker that agrees with the actual medium number.
    for (pattern, title_group, number_group) in
        [(&*FINAL_DISC_MARKER, 1, 2), (&*INITIAL_DISC_MARKER, 2, 1)]
    {
        if let Some(parts) = pattern.captures(title.trim())
            && parts
                .get(number_group)
                .and_then(|number| number.as_str().parse::<u32>().ok())
                == Some(number)
            && let Some(title) = parts.get(title_group).map(|title| title.as_str().trim())
            && !title.is_empty()
        {
            return title;
        }
    }
    title
}

type StoredKeys = BTreeMap<String, BTreeSet<String>>;

pub(super) async fn album_id(
    db: &DatabaseTransaction,
    directory: &str,
    managed_key: Option<&str>,
    file_id: i32,
    incoming: &ReleaseKeys,
    moved_from: Option<&str>,
) -> Result<Uuid> {
    // Tag edits on an existing track leave its release UUID intact.
    let previous = db
        .query_one(
            Query::select()
                .columns(
                    [
                        orm_albums::Column::Id,
                        orm_albums::Column::DirectoryPath,
                        orm_albums::Column::DirectoryScope,
                    ]
                    .map(|c| c.as_column_ref()),
                )
                .from(orm_audio::Entity)
                .inner_join(
                    discs::Entity,
                    column(discs::Column::Id).eq(column(orm_audio::Column::DiscId)),
                )
                .inner_join(
                    orm_albums::Entity,
                    column(orm_albums::Column::Id).eq(column(discs::Column::AlbumId)),
                )
                .and_where(orm_audio::Column::FileId.eq(file_id)),
        )
        .await?;
    let scope = managed_key.map_or_else(|| directory.to_owned(), |key| format!("managed:{key}"));
    if let Some(previous) = previous {
        let old: String = previous.try_get("", "directory_path")?;
        let old_scope: String = previous.try_get("", "directory_scope")?;
        let id = super::uuid(&previous, "id")?;
        if old_scope == scope || managed_key.is_some() {
            return Ok(id);
        }
        if moved_from == Some(old.as_str()) {
            relocate(db, id, directory, None).await?;
            return Ok(id);
        }
        queue(db, &old_scope).await?;
    }

    if managed_key.is_some()
        && let Some(album) = managed_album(db, incoming).await?
    {
        return Ok(album);
    }

    // Read distinct keys instead of loading every track on each import.
    let rows = db
        .query_all(
            Query::select()
                .column(orm_albums::Column::Id.as_column_ref())
                .distinct()
                .columns(
                    [
                        orm_track_release_keys::Column::Field,
                        orm_track_release_keys::Column::Value,
                    ]
                    .map(|c| c.as_column_ref()),
                )
                .from(orm_albums::Entity)
                .left_join(
                    orm_track_release_keys::Entity,
                    column(orm_track_release_keys::Column::AlbumId)
                        .eq(column(orm_albums::Column::Id)),
                )
                .and_where(orm_albums::Column::DirectoryScope.eq(scope))
                .and_where(has_disc(column(orm_albums::Column::Id))),
        )
        .await?;
    let mut releases = BTreeMap::<Uuid, StoredKeys>::new();
    for row in rows {
        let keys = releases.entry(super::uuid(&row, "id")?).or_default();
        if let Some(field) = row.try_get::<Option<String>>("", "field")? {
            keys.entry(field)
                .or_default()
                .insert(row.try_get("", "value")?);
        }
    }

    if let BestMatch::Unique(album) = best_release(&releases, incoming) {
        return Ok(album);
    }

    // A grouped but absent edition can be restored from a unique exact release ID.
    if let Some(mbid) = incoming.0.get(RELEASE) {
        let candidates = db
            .query_all(
                editions_with_mbid(mbid).and_where(has_disc(column(orm_albums::Column::Id)).not()),
            )
            .await?;
        if let [row] = candidates.as_slice() {
            let id = super::uuid(row, "id")?;
            relocate(db, id, directory, managed_key).await?;
            return Ok(id);
        }
    }
    create_album(db, directory, managed_key).await
}

pub(super) async fn managed_key(
    db: &impl ConnectionTrait,
    audio: Uuid,
    keys: &ReleaseKeys,
) -> Result<String> {
    if let Some(mbid) = keys.0.get(RELEASE) {
        return Ok(format!("mbid:{mbid}"));
    }
    // Cache filenames and directories are transport addresses. Use tagged album evidence only.
    if let (Some(title), Some(artist)) = (keys.0.get(TITLE), keys.0.get(ARTIST)) {
        let key = blake3::hash(format!("{title}\0{artist}").as_bytes());
        return Ok(format!("tagged:{key}"));
    }
    if keys.0.contains_key(TITLE)
        && let Some(album) = db
            .query_one(
                Query::select()
                    .expr_as(column(track_album_sources::Column::AlbumUuid), "id")
                    .from(track_album_sources::Entity)
                    .and_where(track_album_sources::Column::TrackUuid.eq(audio.to_string())),
            )
            .await?
    {
        return Ok(format!("catalog:{}", super::uuid(&album, "id")?));
    }
    Ok(format!("audio:{audio}"))
}

async fn managed_album(db: &DatabaseTransaction, incoming: &ReleaseKeys) -> Result<Option<Uuid>> {
    let Some(mbid) = incoming.0.get(RELEASE) else {
        return Ok(None);
    };
    let matches = db.query_all(&editions_with_mbid(mbid)).await?;
    match matches.as_slice() {
        [album] => Ok(Some(super::uuid(album, "id")?)),
        _ => Ok(None),
    }
}

fn editions_with_mbid(mbid: &str) -> SelectStatement {
    use orm_albums::Column as A;
    use release_identity::Column as I;
    Query::select()
        .column(A::Id.as_column_ref())
        .from(release_identity::Entity)
        .inner_join(orm_albums::Entity, column(A::Id).eq(column(I::AlbumId)))
        .and_where(I::MusicbrainzId.eq(mbid))
        .and_where(A::NeedsReview.eq(0))
        .limit(2)
        .to_owned()
}

fn has_disc(album: Expr) -> Expr {
    Expr::exists(
        Query::select()
            .column(discs::Column::Id.as_column_ref())
            .from(discs::Entity)
            .and_where(column(discs::Column::AlbumId).eq(album))
            .to_owned(),
    )
}

async fn relocate(
    db: &DatabaseTransaction,
    id: Uuid,
    directory: &str,
    managed_key: Option<&str>,
) -> Result<()> {
    db.execute(
        Query::update()
            .table(orm_albums::Entity)
            .value(orm_albums::Column::DirectoryPath, Expr::val(directory))
            .value(orm_albums::Column::ManagedKey, Expr::val(managed_key))
            .and_where(Expr::col(orm_albums::Column::Id).eq(Expr::val(id.to_string()))),
    )
    .await?;
    Ok(())
}

async fn create_album(
    db: &DatabaseTransaction,
    directory: &str,
    managed_key: Option<&str>,
) -> Result<Uuid> {
    let id = Uuid::new_v4();
    db.execute(
        Query::insert()
            .into_table(orm_albums::Entity)
            .columns([
                orm_albums::Column::Id,
                orm_albums::Column::DirectoryPath,
                orm_albums::Column::ManagedKey,
            ])
            .values_panic([
                Expr::val(id.to_string()),
                Expr::val(directory.to_owned()),
                Expr::val(managed_key),
            ]),
    )
    .await?;
    super::lifecycle::allocate_handle(db, BrowseKind::Album, &id.to_string()).await?;
    Ok(id)
}

enum BestMatch<T> {
    None,
    Unique(T),
    Ambiguous,
}

fn best_release<T: Copy>(
    releases: &BTreeMap<T, StoredKeys>,
    incoming: &ReleaseKeys,
) -> BestMatch<T> {
    let mut best = BestMatch::None;
    let mut best_rank = None;
    for (id, keys) in releases {
        if let Some(rank) = match_rank(keys, incoming) {
            match best_rank {
                Some(current) if rank < current => (),
                Some(current) if rank == current => best = BestMatch::Ambiguous,
                _ => {
                    best_rank = Some(rank);
                    best = BestMatch::Unique(*id);
                }
            }
        }
    }
    best
}

fn match_rank(stored: &StoredKeys, incoming: &ReleaseKeys) -> Option<(u8, bool)> {
    let same_release = incoming
        .0
        .get(RELEASE)
        .is_some_and(|id| stored.get(RELEASE).is_some_and(|ids| ids.contains(id)));
    let mut score = 0;
    let mut exact = true;
    for (kind, values) in stored {
        // Other media in the same release are compatible with the incoming disc.
        let Some(value) = incoming.0.get(kind) else {
            exact &= kind.starts_with(MEDIUM_PREFIX);
            continue;
        };
        if !values.contains(value) {
            if same_release && matches!(kind.as_str(), TITLE | ARTIST | CATALOG) {
                exact = false;
                continue;
            }
            return None;
        }
        score += key_weight(kind);
    }
    exact &= incoming.0.keys().all(|kind| stored.contains_key(kind));
    Some((score, exact))
}

fn key_weight(kind: &str) -> u8 {
    match kind {
        RELEASE => 16,
        CATALOG | VERSION => 4,
        TITLE | ARTIST => 2,
        _ => 1,
    }
}

pub(super) async fn queue(db: &impl ConnectionTrait, directory: &str) -> Result<()> {
    db.execute(
        Query::insert()
            .into_table(orm_dirty_directories::Entity)
            .columns([orm_dirty_directories::Column::DirectoryScope])
            .values_panic([Expr::val(directory.to_owned())])
            .on_conflict(OnConflict::new().do_nothing().to_owned()),
    )
    .await?;
    Ok(())
}

struct Candidate {
    file_id: i32,
    album: Uuid,
    disc: u32,
    keys: ReleaseKeys,
}

impl Candidate {
    fn strength(&self) -> u8 {
        self.keys.0.keys().map(|key| key_weight(key)).sum()
    }
}

/// Finish each changed directory from its complete set of committed evidence.
pub(super) async fn reconcile(
    db: &DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    loop {
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            return Ok(());
        }
        // Lock before reading evidence so a newer scan cannot race the reassignment.
        let tx = db.begin().await?;
        store.lock(&tx).await?;
        let Some(directory) = tx
            .query_one(
                Query::select()
                    .column(orm_dirty_directories::Column::DirectoryScope)
                    .from(orm_dirty_directories::Entity)
                    .order_by(orm_dirty_directories::Column::DirectoryScope, Order::Asc)
                    .limit(1),
            )
            .await?
        else {
            tx.rollback().await?;
            return Ok(());
        };
        let directory: String = directory.try_get("", "directory_scope")?;
        refresh_scope(&tx, &directory).await?;
        tx.commit().await?;
    }
}

pub(super) async fn refresh_scope(db: &DatabaseTransaction, scope: &str) -> Result<()> {
    reconcile_directory(db, scope).await?;
    super::tracks::reconcile(db).await?;
    super::summary::refresh_directory(db, scope).await?;
    db.execute(
        Query::delete()
            .from_table(orm_dirty_directories::Entity)
            .and_where(
                Expr::col(orm_dirty_directories::Column::DirectoryScope).eq(Expr::val(scope)),
            ),
    )
    .await?;
    Ok(())
}

async fn reconcile_directory(db: &DatabaseTransaction, directory: &str) -> Result<()> {
    let mut tracks = directory_candidates(db, directory).await?;
    // Seed editions with stronger evidence. Equal evidence has the same ordering on every walk.
    tracks.sort_by(|left, right| {
        right
            .strength()
            .cmp(&left.strength())
            .then_with(|| left.keys.cmp(&right.keys))
    });
    let mut groups = Vec::<(bool, Vec<Candidate>)>::new();
    let mut evidence = BTreeMap::<usize, StoredKeys>::new();
    let mut uncertain = BTreeMap::new();
    for track in tracks {
        // Pending tracks share a row only with identical evidence and never seed editions.
        let group = match best_release(&evidence, &track.keys) {
            BestMatch::Unique(group) => group,
            BestMatch::None => {
                groups.push((false, Vec::new()));
                groups.len() - 1
            }
            BestMatch::Ambiguous => *uncertain.entry(track.keys.clone()).or_insert_with(|| {
                groups.push((true, Vec::new()));
                groups.len() - 1
            }),
        };
        if !groups[group].0 {
            let stored = evidence.entry(group).or_default();
            for (kind, value) in &track.keys.0 {
                stored
                    .entry(kind.clone())
                    .or_default()
                    .insert(value.clone());
            }
        }
        groups[group].1.push(track);
    }

    // Reserve an existing UUID owned by this edition before moving any of its tracks.
    let mut reserved = BTreeSet::new();
    for (pending, group) in groups {
        let album = match retained_album(&group, &reserved) {
            Some(id) => id,
            None => {
                let location = db
                    .query_one(
                        Query::select()
                            .columns([
                                orm_albums::Column::DirectoryPath,
                                orm_albums::Column::ManagedKey,
                            ])
                            .from(orm_albums::Entity)
                            .and_where(
                                Expr::col(orm_albums::Column::DirectoryScope)
                                    .eq(Expr::val(directory)),
                            )
                            .limit(1),
                    )
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("release scope disappeared"))?;
                let path: String = location.try_get("", "directory_path")?;
                let managed: Option<String> = location.try_get("", "managed_key")?;
                create_album(db, &path, managed.as_deref()).await?
            }
        };
        reserved.insert(album);
        db.execute(
            Query::update()
                .table(orm_albums::Entity)
                .value(orm_albums::Column::NeedsReview, Expr::val(pending))
                .and_where(Expr::col(orm_albums::Column::Id).eq(Expr::val(album.to_string())))
                .and_where(Expr::col(orm_albums::Column::NeedsReview).ne(Expr::val(pending))),
        )
        .await?;
        for track in group.into_iter().filter(|track| track.album != album) {
            assign_track(db, album, track).await?;
        }
    }
    Ok(())
}

fn retained_album(group: &[Candidate], reserved: &BTreeSet<Uuid>) -> Option<Uuid> {
    let mut votes = BTreeMap::<Uuid, (u8, usize)>::new();
    for track in group {
        if reserved.contains(&track.album) {
            continue;
        }
        let entry = votes.entry(track.album).or_default();
        entry.0 = std::cmp::max(entry.0, track.strength());
        entry.1 += 1;
    }
    votes
        .into_iter()
        .max_by_key(|(id, score)| (*score, Reverse(*id)))
        .map(|(id, _)| id)
}

async fn directory_candidates(db: &DatabaseTransaction, directory: &str) -> Result<Vec<Candidate>> {
    use orm_albums::Column as A;
    use orm_audio::Column as T;
    use orm_track_release_keys::Column as K;
    let rows = db
        .query_all(
            Query::select()
                .column(T::FileId.as_column_ref())
                .column(A::Id.as_column_ref())
                .column(discs::Column::DiscNumber.as_column_ref())
                .columns([K::Field, K::Value].map(|c| c.as_column_ref()))
                .from(orm_albums::Entity)
                .inner_join(
                    discs::Entity,
                    column(discs::Column::AlbumId).eq(column(A::Id)),
                )
                .inner_join(
                    orm_audio::Entity,
                    column(T::DiscId).eq(column(discs::Column::Id)),
                )
                .left_join(
                    orm_track_release_keys::Entity,
                    column(K::FileId).eq(column(T::FileId)),
                )
                .and_where(A::DirectoryScope.eq(directory))
                .order_by(T::FileId.as_column_ref(), Order::Asc),
        )
        .await?;

    // The left join includes files with no usable release tags.
    let mut tracks: Vec<Candidate> = Vec::new();
    for row in rows {
        let file_id = row.try_get("", "file_id")?;
        if tracks.last().is_none_or(|track| track.file_id != file_id) {
            tracks.push(Candidate {
                file_id,
                album: super::uuid(&row, "id")?,
                disc: u32::try_from(row.try_get::<i64>("", "disc_number")?)?,
                keys: ReleaseKeys::default(),
            });
        }
        if let Some(track) = tracks.last_mut()
            && let Some(field) = row.try_get::<Option<String>>("", "field")?
        {
            track.keys.0.insert(field, row.try_get("", "value")?);
        }
    }
    Ok(tracks)
}

async fn assign_track(db: &DatabaseTransaction, album: Uuid, track: Candidate) -> Result<()> {
    let disc = super::ensure_disc(db, album, track.disc).await?;
    super::tracks::prepare_move(db, track.file_id, &disc).await?;
    // Move evidence before the old disc's removal can cascade into its release.
    store(db, track.file_id, album, track.keys).await?;
    db.execute(
        Query::update()
            .table(orm_audio::Entity)
            .value(orm_audio::Column::DiscId, Expr::val(disc))
            .and_where(Expr::col(orm_audio::Column::FileId).eq(Expr::val(track.file_id))),
    )
    .await?;
    Ok(())
}

pub(super) async fn store(
    db: &DatabaseTransaction,
    file_id: i32,
    album: Uuid,
    keys: ReleaseKeys,
) -> Result<()> {
    db.execute(
        Query::delete()
            .from_table(orm_track_release_keys::Entity)
            .and_where(Expr::col(orm_track_release_keys::Column::FileId).eq(Expr::val(file_id))),
    )
    .await?;
    if keys.0.is_empty() {
        return Ok(());
    }

    // The covering index provides distinct edition keys from their source rows.
    use orm_track_release_keys::Column as K;
    let mut insert = Query::insert();
    insert.into_table(orm_track_release_keys::Entity).columns([
        K::FileId,
        K::AlbumId,
        K::Field,
        K::Value,
    ]);
    let album = album.to_string();
    for (field, value) in keys.0 {
        insert.values([
            Expr::val(file_id),
            Expr::val(&album),
            Expr::val(field),
            Expr::val(value),
        ])?;
    }
    db.execute(&insert).await?;
    Ok(())
}
