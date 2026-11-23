//! Adapt UUID-backed identities to the collection API's existing integer handles.

use anyhow::Result;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, FromQueryResult, Statement, Value};

use super::collection::CollectionQueryListMode;

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Album,
    Artist,
}

impl Kind {
    fn table(self) -> &'static str {
        match self {
            Self::Album => "albums",
            Self::Artist => "artists",
        }
    }

    fn view(self) -> &'static str {
        match self {
            Self::Album => "music_metadata_browse_albums",
            Self::Artist => "music_metadata_browse_artists",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Artist => "artist",
        }
    }

    fn identities(self) -> &'static str {
        match self {
            Self::Album => "SELECT id, title AS name FROM music_metadata_current_album_titles",
            Self::Artist => {
                "SELECT a.id, COALESCE(target.name, a.name) AS name FROM music_metadata_artists a
                LEFT JOIN music_metadata_artist_redirects r ON r.source_uuid = a.id
                LEFT JOIN music_metadata_artists target ON target.id = r.target_uuid"
            }
        }
    }

    pub(super) fn old_operator(self) -> &'static str {
        match self {
            Self::Album => "lib::album",
            Self::Artist => "lib::artist",
        }
    }

    fn uuid_operator(self) -> &'static str {
        match self {
            Self::Album => "lib::release",
            Self::Artist => "lib::artist_id",
        }
    }
}

fn statement(sql: String, binds: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::Sqlite, sql, binds)
}

fn browse_sql(kind: Kind) -> String {
    let table = kind.table();
    let view = kind.view();
    // Reuse the name-derived legacy alphabetic bucket when that name was indexed already.
    format!(
        "SELECT b.id, b.name, b.source_uuid, COALESCE(old.\"group\",
            CASE WHEN upper(substr(b.name,1,1)) GLOB '[A-Z]' THEN upper(substr(b.name,1,1)) ELSE '#' END
         ) AS \"group\" FROM {view} b LEFT JOIN {table} old ON old.name = b.name"
    )
}

pub(super) async fn portable_clause(
    db: &impl ConnectionTrait,
    operator_name: String,
    parameter: String,
) -> Result<(String, String)> {
    let kind = match operator_name.as_str() {
        "lib::album" => Kind::Album,
        "lib::artist" => Kind::Artist,
        _ => return Ok((operator_name, parameter)),
    };
    if let Ok(id) = parameter.parse::<i32>()
        && id < 0
    {
        let row = db
            .query_one_raw(statement(
                "SELECT source_uuid FROM music_metadata_browse_handles WHERE kind = ? AND id = ?"
                    .into(),
                [kind.label().into(), (-i64::from(id)).into()],
            ))
            .await?
            .ok_or(crate::error::ExpectedError::NotFound { what: "collection" })?;
        let source: String = row.try_get("", "source_uuid")?;
        let parameter = match kind {
            Kind::Album => super::resolved_metadata::edits::portable_edition(db, source.parse()?)
                .await?
                .to_string(),
            Kind::Artist => source,
        };
        return Ok((kind.uuid_operator().into(), parameter));
    }
    Ok((operator_name, parameter))
}

pub(super) async fn editable_clause(
    db: &impl ConnectionTrait,
    view: &mut super::mixes::MixQueryView,
) -> Result<()> {
    let kind = match view.operator.as_str() {
        "lib::release" => Kind::Album,
        "lib::artist_id" => Kind::Artist,
        _ => return Ok(()),
    };
    // The existing editor uses integer handles. Keep the stored clause portable across devices.
    let identity = match kind {
        Kind::Album => {
            "h.source_uuid IN (SELECT album_id FROM music_metadata_edition_matches
                        WHERE edition_uuid = ?2 UNION ALL SELECT album_id FROM music_metadata_album_track_anchors
                        WHERE track_uuid = ?2)"
        }
        Kind::Artist => "h.source_uuid = ?2",
    };
    let row = db
        .query_one_raw(statement(
            format!(
                "SELECT -h.id AS id FROM music_metadata_browse_handles h
                 JOIN ({}) a ON a.id = h.source_uuid WHERE h.kind = ?1 AND {identity}",
                kind.identities()
            ),
            [kind.label().into(), view.parameter.clone().into()],
        ))
        .await?;
    if let Some(row) = row {
        view.operator = kind.old_operator().into();
        view.parameter = row.try_get::<i32>("", "id")?.to_string();
    } else {
        view.available = false;
    }
    Ok(())
}

pub(super) async fn counts(db: &DatabaseConnection, kind: Kind) -> Result<Vec<(String, i32)>> {
    if matches!(kind, Kind::Album) {
        super::resolved_metadata::ensure_tracks_current(db).await?;
    }
    let rows = db
        .query_all_raw(statement(
            format!(
                "SELECT \"group\", count(*) AS n FROM ({}) GROUP BY \"group\" ORDER BY \"group\"",
                browse_sql(kind)
            ),
            [],
        ))
        .await?;
    rows.iter()
        .map(|r| Ok((r.try_get("", "group")?, r.try_get("", "n")?)))
        .collect()
}

pub(super) async fn groups<T: FromQueryResult>(
    db: &DatabaseConnection,
    kind: Kind,
    groups: &[String],
) -> Result<Vec<T>> {
    if matches!(kind, Kind::Album) {
        super::resolved_metadata::ensure_tracks_current(db).await?;
    }
    if groups.is_empty() {
        return Ok(Vec::new());
    }
    let slots = vec!["?"; groups.len()].join(", ");
    Ok(T::find_by_statement(statement(
        format!(
            "SELECT * FROM ({}) WHERE \"group\" IN ({slots}) ORDER BY name COLLATE NOCASE, id",
            browse_sql(kind)
        ),
        groups.iter().cloned().map(Into::into),
    ))
    .all(db)
    .await?)
}

pub(super) async fn by_id<T: FromQueryResult>(
    db: &DatabaseConnection,
    kind: Kind,
    ids: &[i32],
) -> Result<Vec<T>> {
    let mut found = Vec::new();
    // Positive IDs from previously saved navigation remain resolvable after the browse switch.
    for ids in ids.chunks(400) {
        let slots = vec!["?"; ids.len()].join(", ");
        found.extend(T::find_by_statement(statement(
            format!("SELECT -h.id AS id, a.name, a.id AS source_uuid, COALESCE(old.\"group\", '#') AS \"group\"
                FROM music_metadata_browse_handles h JOIN ({}) a ON a.id = h.source_uuid
                LEFT JOIN {table} old ON old.name = a.name
                WHERE h.kind = '{}' AND h.id IN ({})
                UNION ALL SELECT id, name, source_uuid, \"group\" FROM {table} WHERE id IN ({slots})",
                kind.identities(), kind.label(), vec!["-?"; ids.len()].join(", "), table = kind.table()),
            ids.iter().chain(ids).copied().map(Into::into),
        )).all(db).await?);
    }
    Ok(found)
}

pub(super) async fn list<T: FromQueryResult>(
    db: &DatabaseConnection,
    kind: Kind,
    limit: u64,
    mode: CollectionQueryListMode,
) -> Result<Vec<T>> {
    if matches!(kind, Kind::Album) {
        super::resolved_metadata::ensure_tracks_current(db).await?;
    }
    let order = match mode {
        CollectionQueryListMode::Forward => "abs(id)",
        CollectionQueryListMode::Reverse => "abs(id) DESC",
        CollectionQueryListMode::Random => "random()",
        CollectionQueryListMode::Name => "name COLLATE NOCASE, id",
    };
    Ok(T::find_by_statement(statement(
        format!(
            "SELECT * FROM ({}) ORDER BY {order} LIMIT ?",
            browse_sql(kind)
        ),
        [i64::try_from(limit)?.into()],
    ))
    .all(db)
    .await?)
}

pub(super) async fn resolve_search(
    db: &DatabaseConnection,
    kind: Kind,
    query: &str,
    legacy: &[i64],
    limit: usize,
) -> Result<Vec<i64>> {
    if matches!(kind, Kind::Album) {
        super::resolved_metadata::ensure_tracks_current(db).await?;
    }
    let pattern = query
        .trim_end_matches('*')
        .split_whitespace()
        .map(|term| {
            term.replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_")
        })
        .collect::<Vec<_>>()
        .join("%");
    let pattern = format!("%{pattern}%");
    // Match the current metadata and former artist names alongside the legacy FTS results.
    let aliases = match kind {
        Kind::Album => "",
        Kind::Artist => "OR EXISTS (
            SELECT 1 FROM music_metadata_artist_redirects r JOIN music_metadata_artists old ON old.id = r.source_uuid
            WHERE r.target_uuid = b.source_uuid AND old.name LIKE ?1 ESCAPE '!')",
    };
    let rows = db.query_all_raw(statement(
        format!("SELECT b.id FROM {} b WHERE b.name LIKE ?1 ESCAPE '!' {aliases} ORDER BY b.name, b.id LIMIT ?2", kind.view()),
        [pattern.into(), i64::try_from(limit)?.into()],
    )).await?;
    let mut ids = rows
        .iter()
        .map(|row| row.try_get::<i64>("", "id"))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut seen: std::collections::BTreeSet<_> = ids.iter().copied().collect();
    // Preserve FTS matches on transliterations and legacy names after switching to UUID handles.
    for old in legacy.chunks(400) {
        let slots = vec!["?"; old.len()].join(", ");
        let current = legacy_matches(kind, &slots);
        let rows = db
            .query_all_raw(statement(
                format!(
                    "SELECT id FROM {} WHERE id IN ({slots}) UNION {current}",
                    kind.view()
                ),
                old.iter().chain(old).copied().map(Into::into),
            ))
            .await?;
        for row in rows {
            let id = row.try_get("", "id")?;
            if ids.len() < limit && seen.insert(id) {
                ids.push(id);
            }
        }
    }
    Ok(ids)
}

fn legacy_matches(kind: Kind, slots: &str) -> String {
    match kind {
        Kind::Album => format!(
            "SELECT b.id FROM albums a JOIN media_file_albums l ON l.album_uuid = a.source_uuid
             JOIN music_metadata_audio t ON t.id = l.track_uuid JOIN music_metadata_discs d ON d.id = t.disc_id
             JOIN music_metadata_browse_albums b ON b.source_uuid = d.album_id WHERE a.id IN ({slots})"
        ),
        Kind::Artist => format!(
            "SELECT b.id FROM artists a LEFT JOIN music_metadata_artist_redirects legacy ON legacy.source_uuid = a.source_uuid
             LEFT JOIN media_file_artists l ON l.artist_uuid = a.source_uuid
             LEFT JOIN music_metadata_audio t ON t.id = l.track_uuid
             LEFT JOIN music_metadata_credits c ON c.file_id = t.file_id AND c.scope = 2
                 AND c.role = 'Primary Artist' AND c.credited_as = a.name COLLATE NOCASE
             LEFT JOIN music_metadata_artist_redirects r ON r.source_uuid = c.artist_id
             JOIN music_metadata_browse_artists b
                 ON b.source_uuid = COALESCE(r.target_uuid, c.artist_id, legacy.target_uuid, a.source_uuid)
             WHERE a.id IN ({slots})"
        ),
    }
}

macro_rules! resolved_browse_query {
    ($model:ty, $kind:ident) => {
        #[async_trait::async_trait]
        impl super::collection::CollectionQuery for $model {
            fn collection_type() -> super::collection::CollectionQueryType {
                super::collection::CollectionQueryType::$kind
            }

            async fn query_builder(
                _db: &crate::connection::MainDbConnection,
                id: i32,
            ) -> anyhow::Result<Vec<(String, String)>> {
                Ok(vec![(
                    super::browse::Kind::$kind.old_operator().into(),
                    id.to_string(),
                )])
            }

            async fn count_by_first_letter(
                db: &crate::connection::MainDbConnection,
            ) -> anyhow::Result<Vec<(String, i32)>> {
                super::browse::counts(db, super::browse::Kind::$kind).await
            }

            async fn get_groups(
                db: &crate::connection::MainDbConnection,
                groups: Vec<String>,
            ) -> anyhow::Result<Vec<(String, Vec<(Self, std::collections::HashSet<i32>)>)>> {
                let items: Vec<Self> =
                    super::browse::groups(db, super::browse::Kind::$kind, &groups).await?;
                let mut buckets = std::collections::BTreeMap::<_, Vec<_>>::new();
                for item in items {
                    buckets
                        .entry(item.group.clone())
                        .or_default()
                        .push((item, Default::default()));
                }
                Ok(groups
                    .into_iter()
                    .map(|group| {
                        let items = buckets.remove(&group).unwrap_or_default();
                        (group, items)
                    })
                    .collect())
            }

            async fn get_by_ids(
                db: &crate::connection::MainDbConnection,
                ids: &[i32],
            ) -> anyhow::Result<Vec<Self>> {
                super::browse::by_id(db, super::browse::Kind::$kind, ids).await
            }

            async fn list(
                db: &crate::connection::MainDbConnection,
                limit: u64,
                mode: super::collection::CollectionQueryListMode,
            ) -> anyhow::Result<Vec<Self>> {
                super::browse::list(db, super::browse::Kind::$kind, limit, mode).await
            }

            fn id(&self) -> i32 {
                self.id
            }
            fn name(&self) -> &str {
                &self.name
            }
            fn readonly(&self) -> bool {
                false
            }
        }
    };
}

pub(super) use resolved_browse_query;
