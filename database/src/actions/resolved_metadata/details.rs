use crate::actions::resolved_metadata::queries::{
    TRACK_RECORDINGS, field, track_recordings, verified_audio,
};
use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use anyhow::Result;
use metadata::{
    model::{
        AlbumExtraResource, AlbumExtraResourceType, Artwork, MetadataBase, MetadataLink,
        MetadataProvenance, ReleaseSecondaryType,
    },
    resolve::LocalMetadata,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, ExprTrait, JoinType, QueryResult, Value,
    sea_query::{Expr, IntoTableRef, Order, Query, SelectStatement, UnionType},
};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::entities::music_metadata::{
    audio, audio as verified, details as orm_details, discs, track_assets, values as orm_values,
};

use super::{CreditScope, Winners, column, credits};

const ALBUM: i32 = CreditScope::Album as i32;
const DISC: i32 = CreditScope::Disc as i32;
const AUDIO: i32 = CreditScope::Audio as i32;

#[derive(Clone, Copy)]
pub(super) enum Selection {
    Album(Uuid),
    Disc(Uuid),
    Audio(i32),
    Recording(Uuid),
}

impl Selection {
    fn scope(self) -> i32 {
        match self {
            Self::Album(_) => ALBUM,
            Self::Disc(_) => DISC,
            Self::Audio(_) | Self::Recording(_) => AUDIO,
        }
    }

    // Read side tables in release, track, and tag order without loading audio metadata.
    pub(super) fn rows(
        self,
        table: impl IntoTableRef,
        file_id: impl ColumnTrait,
        scope: impl ColumnTrait,
    ) -> SelectStatement {
        let mut query = side_rows(table, file_id, scope, self.scope());
        let scope = match self {
            Self::Album(id) => discs::Column::AlbumId.eq(id.to_string()),
            Self::Disc(id) => audio::Column::DiscId.eq(id.to_string()),
            Self::Audio(id) => audio::Column::FileId.eq(id),
            Self::Recording(id) => {
                let id = id.to_string();
                let tracks = Query::select()
                    .column(verified::Column::FileId.as_column_ref())
                    .from(track_recordings())
                    .inner_join(
                        track_assets::Entity,
                        column(track_assets::Column::TrackId)
                            .eq(field(TRACK_RECORDINGS, "track_id")),
                    )
                    .inner_join(
                        verified_audio(),
                        column(verified::Column::FileId).eq(column(track_assets::Column::FileId)),
                    )
                    .and_where(field(TRACK_RECORDINGS, "recording_id").eq(&id))
                    .and_where(track_assets::Column::EndMs.is_null())
                    .to_owned();

                // Whole-file versions contribute recording metadata in either identity path.
                let files = Query::select()
                    .column(verified::Column::FileId)
                    .from(verified_audio())
                    .and_where(verified::Column::RecordingId.eq(id))
                    .union(UnionType::Distinct, tracks)
                    .to_owned();
                column(audio::Column::FileId).in_subquery(files)
            }
        };
        query.and_where(scope);
        query
    }
}

fn side_rows(
    table: impl IntoTableRef,
    file_id: impl ColumnTrait,
    scope: impl ColumnTrait,
    selection: i32,
) -> SelectStatement {
    Query::select()
        .column(file_id.as_column_ref())
        .from(table)
        .join(
            JoinType::InnerJoin,
            audio::Entity,
            column(audio::Column::FileId).eq(column(file_id)),
        )
        .join(
            JoinType::InnerJoin,
            discs::Entity,
            column(discs::Column::Id).eq(column(audio::Column::DiscId)),
        )
        .and_where(scope.eq(selection))
        .order_by(discs::Column::DiscNumber.as_column_ref(), Order::Asc)
        .order_by_expr(audio::Column::TrackNumber.is_null(), Order::Asc)
        .order_by(audio::Column::TrackNumber.as_column_ref(), Order::Asc)
        .order_by(audio::Column::FileId.as_column_ref(), Order::Asc)
        .to_owned()
}

pub(super) type Contributors = BTreeMap<i32, BTreeSet<String>>;

#[derive(Default)]
pub(super) struct AlbumLists {
    pub labels: Option<Vec<String>>,
    pub extra_resources: Option<Vec<AlbumExtraResource>>,
    pub secondary_types: BTreeMap<ReleaseSecondaryType, bool>,
}

struct Values<'a> {
    scope: i32,
    rows: &'a mut Vec<Vec<Value>>,
}

impl Values<'_> {
    fn add<'a>(&mut self, field: &str, qualifier: &str, values: impl IntoIterator<Item = &'a str>) {
        for (position, value) in values.into_iter().enumerate() {
            self.rows.push(vec![
                self.scope.into(),
                field.to_owned().into(),
                qualifier.to_owned().into(),
                (position as i64).into(),
                value.to_owned().into(),
            ]);
        }
    }

    fn list(&mut self, field: &str, values: Option<&[String]>) {
        self.add(field, "", values.into_iter().flatten().map(String::as_str));
    }

    fn shared(&mut self, base: &MetadataBase) {
        self.list("genres", base.genres.as_deref());
        self.list("moods", base.moods.as_deref());
        self.list("languages", base.languages.as_deref());
        self.list("user_tags", base.user_tags.as_deref());
        for (qualifier, values) in base.identifiers.iter().flatten() {
            self.add("identifiers", qualifier, values.iter().map(String::as_str));
        }
    }
}

pub(super) async fn store(
    db: &DatabaseTransaction,
    file_id: i32,
    metadata: &LocalMetadata,
) -> Result<()> {
    let mut rows = Vec::new();
    for (scope, base) in [
        (ALBUM, &metadata.album.base),
        (DISC, &metadata.disc.base),
        (AUDIO, &metadata.audio.base),
    ] {
        Values {
            scope,
            rows: &mut rows,
        }
        .shared(base);
    }
    {
        let mut values = Values {
            scope: ALBUM,
            rows: &mut rows,
        };
        values.list("labels", metadata.album.labels.as_deref());
        for (&kind, &enabled) in &metadata.album.release_secondary_types {
            values.add(
                "release_secondary_types",
                kind.as_str(),
                [if enabled { "1" } else { "0" }],
            );
        }
        values.add(
            "extra_resources",
            "scans",
            metadata
                .album
                .extra_resources
                .iter()
                .flatten()
                .map(|resource| resource.path.as_str()),
        );
    }
    for (algorithm, value) in metadata.audio.fingerprints.iter().flatten() {
        Values {
            scope: AUDIO,
            rows: &mut rows,
        }
        .add("fingerprints", algorithm, [value.as_str()]);
    }

    // Replace ordered tag values as a batch, keeping below SQLite's bind limit.
    db.execute(
        Query::delete()
            .from_table(orm_values::Entity)
            .and_where(Expr::col(orm_values::Column::FileId).eq(Expr::val(file_id))),
    )
    .await?;
    for rows in rows.chunks(128) {
        use orm_values::Column as C;
        let mut insert = Query::insert();
        insert.into_table(orm_values::Entity).columns([
            C::FileId,
            C::Scope,
            C::Field,
            C::Qualifier,
            C::Position,
            C::Value,
        ]);
        for row in rows {
            insert.values(
                std::iter::once(Expr::val(file_id)).chain(row.iter().cloned().map(Expr::val)),
            )?;
        }
        db.execute(&insert).await?;
    }

    // Keep the structured, rarely filtered fields away from the scalar evidence.
    db.execute(
        Query::delete()
            .from_table(orm_details::Entity)
            .and_where(Expr::col(orm_details::Column::FileId).eq(Expr::val(file_id))),
    )
    .await?;
    use orm_details::Column as DetailColumn;
    let mut insert = Query::insert();
    insert.into_table(orm_details::Entity).columns([
        DetailColumn::FileId,
        DetailColumn::Scope,
        DetailColumn::Artwork,
        DetailColumn::Links,
        DetailColumn::Provenance,
    ]);
    let mut has_details = false;
    for (scope, base) in [
        (ALBUM, &metadata.album.base),
        (DISC, &metadata.disc.base),
        (AUDIO, &metadata.audio.base),
    ] {
        if base.artwork.is_some() || base.links.is_some() || base.provenance.is_some() {
            insert.values([
                Expr::val(file_id),
                Expr::val(scope),
                Expr::val(encode(&base.artwork)?),
                Expr::val(encode(&base.links)?),
                Expr::val(encode(&base.provenance)?),
            ])?;
            has_details = true;
        }
    }
    if has_details {
        db.execute(&insert).await?;
    }
    Ok(())
}

fn encode<T: Serialize>(value: &Option<T>) -> Result<Option<String>> {
    Ok(value.as_ref().map(serde_json::to_string).transpose()?)
}

fn decode<T: DeserializeOwned>(row: &QueryResult, field: &str) -> Result<Option<T>> {
    let value: Option<String> = row.try_get("", field)?;
    Ok(value.as_deref().map(serde_json::from_str).transpose()?)
}

#[derive(Default)]
struct SeenObjects {
    artwork: BTreeSet<String>,
    links: BTreeSet<String>,
}

fn decode_unique<T: DeserializeOwned>(
    row: &QueryResult,
    field: &str,
    seen: &mut BTreeSet<String>,
) -> Result<Option<T>> {
    let value: Option<String> = row.try_get("", field)?;
    let Some(value) = value.filter(|value| !seen.contains(value)) else {
        return Ok(None);
    };
    let decoded = serde_json::from_str(&value)?;
    seen.insert(value);
    Ok(Some(decoded))
}

#[derive(Default)]
pub(super) struct Lists {
    pub album: AlbumLists,
    pub fingerprints: Option<BTreeMap<String, String>>,
}

pub(super) async fn load(
    db: &impl ConnectionTrait,
    selection: Selection,
    first_file: Option<i32>,
    base: &mut MetadataBase,
    winners: &Winners,
) -> Result<Lists> {
    use orm_values::Column as ValueColumn;
    let mut query = selection.rows(orm_values::Entity, ValueColumn::FileId, ValueColumn::Scope);
    query
        .columns(
            [
                ValueColumn::Field,
                ValueColumn::Qualifier,
                ValueColumn::Value,
            ]
            .map(|c| c.as_column_ref()),
        )
        .order_by_columns(
            [
                ValueColumn::Field,
                ValueColumn::Qualifier,
                ValueColumn::Position,
            ]
            .map(|c| (c.as_column_ref(), Order::Asc)),
        );
    let rows = db.query_all(&query).await?;
    let mut lists = Lists::default();
    let mut contributors = Contributors::new();
    for row in rows {
        let file_id: i32 = row.try_get("", "file_id")?;
        if first_file.is_some_and(|first| first != file_id) {
            continue;
        }
        add_value(base, &mut lists, &mut contributors, file_id, &row)?;
    }
    credits::load(db, selection, first_file, base, &mut contributors).await?;

    // Artwork and links often repeat unchanged on every track of a release.
    use orm_details::Column as DetailColumn;
    let mut query = selection.rows(
        orm_details::Entity,
        DetailColumn::FileId,
        DetailColumn::Scope,
    );
    query
        .columns([DetailColumn::Artwork, DetailColumn::Links].map(|c| c.as_column_ref()))
        .and_where(
            DetailColumn::Artwork
                .is_not_null()
                .or(DetailColumn::Links.is_not_null()),
        );
    let rows = db.query_all(&query).await?;
    let mut seen = SeenObjects::default();
    for row in rows {
        let file_id: i32 = row.try_get("", "file_id")?;
        add_objects(
            base,
            selection,
            first_file,
            file_id,
            &mut contributors,
            &mut seen,
            &row,
        )?;
    }
    load_provenance(db, selection, base, winners, &contributors).await?;
    Ok(lists)
}

async fn load_provenance(
    db: &impl ConnectionTrait,
    selection: Selection,
    base: &mut MetadataBase,
    winners: &Winners,
    contributors: &Contributors,
) -> Result<()> {
    let ids: BTreeSet<_> = match selection {
        Selection::Audio(id) => BTreeSet::from([id]),
        _ => winners
            .values()
            .chain(contributors.keys())
            .copied()
            .collect(),
    };
    let ids: Vec<_> = ids.into_iter().collect();

    // Fetch JSON only from tracks that contributed a winning field.
    for ids in ids.chunks(500) {
        use orm_details::Column as C;
        let mut query = side_rows(orm_details::Entity, C::FileId, C::Scope, selection.scope());
        query
            .column(C::Provenance.as_column_ref())
            .and_where(C::Provenance.is_not_null())
            .and_where(C::FileId.is_in(ids.iter().copied()));
        let rows = db.query_all(&query).await?;
        append_provenance(base, selection, winners, contributors, rows)?;
    }
    Ok(())
}

fn append_provenance(
    base: &mut MetadataBase,
    selection: Selection,
    winners: &Winners,
    contributors: &Contributors,
    rows: Vec<QueryResult>,
) -> Result<()> {
    for row in rows {
        let file_id: i32 = row.try_get("", "file_id")?;
        let contributing = contributors.get(&file_id);
        let Some(sources) = decode::<MetadataProvenance>(&row, "provenance")? else {
            continue;
        };
        if matches!(selection, Selection::Audio(_)) {
            base.provenance = Some(sources);
            continue;
        }
        let collected = base.provenance.get_or_insert_default();
        for (field, evidence) in sources {
            if winners.get(field.as_str()) == Some(&file_id)
                || contributing.is_some_and(|fields| fields.contains(&field))
            {
                append_unique(collected.entry(field).or_default(), evidence);
            }
        }
    }
    Ok(())
}

fn add_value(
    base: &mut MetadataBase,
    lists: &mut Lists,
    contributors: &mut Contributors,
    file_id: i32,
    row: &QueryResult,
) -> Result<()> {
    let field: String = row.try_get("", "field")?;
    let key: String = row.try_get("", "qualifier")?;
    let value: String = row.try_get("", "value")?;
    let mut source_path = field.clone();
    let inserted = match field.as_str() {
        "genres" => push(base.genres.get_or_insert_default(), value),
        "moods" => push(base.moods.get_or_insert_default(), value),
        "languages" => push(base.languages.get_or_insert_default(), value),
        "user_tags" => push(base.user_tags.get_or_insert_default(), value),

        // Qualified items keep separate role, namespace, and algorithm keys.
        "identifiers" => {
            source_path = format!("{field}.{key}");
            push(
                base.identifiers
                    .get_or_insert_default()
                    .entry(key)
                    .or_default(),
                value,
            )
        }
        "fingerprints" => {
            lists
                .fingerprints
                .get_or_insert_default()
                .insert(key, value);
            true
        }
        "labels" => push(lists.album.labels.get_or_insert_default(), value),
        "release_secondary_types" => {
            source_path = format!("{field}.{key}");
            let kind = ReleaseSecondaryType::from_label(&key)
                .ok_or_else(|| anyhow::anyhow!("unknown secondary release type {key}"))?;
            if let Entry::Vacant(entry) = lists.album.secondary_types.entry(kind) {
                entry.insert(value == "1");
                true
            } else {
                false
            }
        }
        "extra_resources" => push(
            lists.album.extra_resources.get_or_insert_default(),
            AlbumExtraResource {
                kind: AlbumExtraResourceType::Scans,
                path: value,
            },
        ),
        _ => anyhow::bail!("unknown metadata value field {field}"),
    };
    if inserted {
        contributors.entry(file_id).or_default().insert(source_path);
    }
    Ok(())
}

fn add_objects(
    base: &mut MetadataBase,
    selection: Selection,
    first_file: Option<i32>,
    file_id: i32,
    contributors: &mut Contributors,
    seen: &mut SeenObjects,
    row: &QueryResult,
) -> Result<()> {
    // A disc keeps its first available cover. A release combines distinct covers.
    if (!matches!(selection, Selection::Disc(_)) || base.artwork.is_none())
        && let Some(artwork) = decode_unique::<Vec<Artwork>>(row, "artwork", &mut seen.artwork)?
        && append_unique(base.artwork.get_or_insert_default(), artwork)
    {
        contributors
            .entry(file_id)
            .or_default()
            .insert("artwork".into());
    }
    if first_file.is_none_or(|first| first == file_id)
        && let Some(links) = decode_unique::<Vec<MetadataLink>>(row, "links", &mut seen.links)?
        && append_unique(base.links.get_or_insert_default(), links)
    {
        contributors
            .entry(file_id)
            .or_default()
            .insert("links".into());
    }
    Ok(())
}

fn push<T: PartialEq>(values: &mut Vec<T>, value: T) -> bool {
    if values.contains(&value) {
        return false;
    }
    values.push(value);
    true
}

fn append_unique<T: PartialEq>(values: &mut Vec<T>, incoming: Vec<T>) -> bool {
    let previous = values.len();
    for value in incoming {
        push(values, value);
    }
    values.len() > previous
}
