use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use metadata::{
    model::{
        AlbumExtraResource, AlbumExtraResourceType, Artwork, MetadataBase, MetadataLink,
        MetadataProvenance, ReleaseSecondaryType,
    },
    resolve::LocalMetadata,
};
use sea_orm::{ConnectionTrait, DatabaseTransaction, QueryResult, Value};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use super::{CreditScope, Winners, credits, statement};

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
    pub(super) async fn rows(
        self,
        db: &impl ConnectionTrait,
        table: &str,
        fields: &str,
        restriction: &str,
        order: &str,
    ) -> Result<Vec<QueryResult>> {
        let (filter, id) = match self {
            Self::Album(id) => ("d.album_id = ?", id.to_string().into()),
            Self::Disc(id) => ("a.disc_id = ?", id.to_string().into()),
            Self::Audio(id) => ("a.file_id = ?", id.into()),
            Self::Recording(id) => (
                "a.file_id IN (
                SELECT file_id FROM music_metadata_current_recording_audio WHERE recording_id = ?2
                UNION SELECT current.file_id FROM music_metadata_track_recordings r
                JOIN music_metadata_track_assets v ON v.track_id = r.track_id
                JOIN music_metadata_current_recording_audio current ON current.file_id = v.file_id
                WHERE r.recording_id = ?2 AND v.end_ms IS NULL
            )",
                id.to_string().into(),
            ),
        };
        let sql = format!(
            "SELECT v.file_id, {fields} FROM {table} v
             JOIN music_metadata_audio a ON a.file_id = v.file_id
             JOIN music_metadata_discs d ON d.id = a.disc_id
             WHERE v.scope = ? AND {filter}{restriction}
             ORDER BY d.disc_number, a.track_number IS NULL, a.track_number, a.file_id{order}"
        );
        Ok(db
            .query_all_raw(statement(sql, [self.scope().into(), id]))
            .await?)
    }
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
    db.execute_raw(statement(
        "DELETE FROM music_metadata_values WHERE file_id = ?",
        [file_id.into()],
    ))
    .await?;
    for rows in rows.chunks(128) {
        let placeholders = vec!["(?, ?, ?, ?, ?, ?)"; rows.len()].join(", ");
        let sql = format!(
            "INSERT INTO music_metadata_values(file_id, scope, field, qualifier, position, value) VALUES {placeholders}"
        );
        let binds = rows
            .iter()
            .flat_map(|row| std::iter::once(file_id.into()).chain(row.iter().cloned()));
        db.execute_raw(statement(sql, binds)).await?;
    }

    // Keep the structured, rarely filtered fields away from the scalar evidence.
    db.execute_raw(statement(
        "DELETE FROM music_metadata_details WHERE file_id = ?",
        [file_id.into()],
    ))
    .await?;
    let mut details = Vec::new();
    for (scope, base) in [
        (ALBUM, &metadata.album.base),
        (DISC, &metadata.disc.base),
        (AUDIO, &metadata.audio.base),
    ] {
        if base.artwork.is_some() || base.links.is_some() || base.provenance.is_some() {
            details.extend([
                file_id.into(),
                scope.into(),
                encode(&base.artwork)?.into(),
                encode(&base.links)?.into(),
                encode(&base.provenance)?.into(),
            ]);
        }
    }
    if !details.is_empty() {
        let placeholders = vec!["(?, ?, ?, ?, ?)"; details.len() / 5].join(", ");
        let sql = format!("INSERT INTO music_metadata_details VALUES {placeholders}");
        db.execute_raw(statement(sql, details)).await?;
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
    let rows = selection
        .rows(
            db,
            "music_metadata_values",
            "v.field, v.qualifier, v.value",
            "",
            ", v.field, v.qualifier, v.position",
        )
        .await?;
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
    let rows = selection
        .rows(
            db,
            "music_metadata_details",
            "v.artwork, v.links",
            " AND (v.artwork IS NOT NULL OR v.links IS NOT NULL)",
            "",
        )
        .await?;
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
        let parameters = vec!["?"; ids.len()].join(", ");
        let sql = format!(
            "SELECT v.file_id, v.provenance FROM music_metadata_details v
             JOIN music_metadata_audio a ON a.file_id = v.file_id
             JOIN music_metadata_discs d ON d.id = a.disc_id
             WHERE v.scope = ? AND v.provenance IS NOT NULL AND v.file_id IN ({parameters})
             ORDER BY d.disc_number, a.track_number IS NULL, a.track_number, v.file_id"
        );
        let values =
            std::iter::once(selection.scope().into()).chain(ids.iter().map(|id| (*id).into()));
        let rows = db.query_all_raw(statement(sql, values)).await?;
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
            if let std::collections::btree_map::Entry::Vacant(entry) =
                lists.album.secondary_types.entry(kind)
            {
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
