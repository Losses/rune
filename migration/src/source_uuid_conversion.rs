//! Transactional conversion of source relationships to stable UUIDs.
use sea_orm_migration::{
    prelude::*,
    sea_orm::{ConnectionTrait, DatabaseTransaction, Value, prelude::Uuid},
};
use std::collections::{BTreeMap, HashMap, HashSet};

fn invalid(message: impl Into<String>) -> DbErr {
    DbErr::Custom(message.into())
}
fn pragma(function: &str, name: &str) -> SelectStatement {
    Query::select()
        .column(Asterisk)
        .from_function(
            Func::cust(Alias::new(function)).arg(Expr::val(name)),
            Alias::new("schema_info"),
        )
        .to_owned()
}
async fn rows(
    tx: &DatabaseTransaction,
    table: &str,
    cols: &[&str],
) -> Result<Vec<sea_orm_migration::sea_orm::QueryResult>, DbErr> {
    let q = Query::select()
        .columns(cols.iter().map(|c| Alias::new(*c)))
        .from(Alias::new(table))
        .order_by(Alias::new("id"), Order::Asc)
        .to_owned();
    tx.query_all(&q).await
}
pub(super) async fn identities(
    tx: &DatabaseTransaction,
    table: &str,
) -> Result<HashMap<i32, String>, DbErr> {
    let schema = SchemaManager::new(tx);
    if !schema.has_table(table).await? {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    let mut seen = HashSet::new();
    for row in rows(tx, table, &["id", "hlc_uuid"]).await? {
        let id: i32 = row.try_get("", "id")?;
        let old: String = row.try_get("", "hlc_uuid")?;
        let parsed = Uuid::parse_str(&old)
            .ok()
            .filter(|uuid| !uuid.is_nil() && !seen.contains(uuid));
        let uuid = match parsed {
            Some(uuid) => uuid,
            None => {
                ensure_unpublished(tx, table, id).await?;
                loop {
                    let uuid = Uuid::new_v4();
                    if !seen.contains(&uuid) {
                        break uuid;
                    }
                }
            }
        };
        seen.insert(uuid);
        let canonical = uuid.to_string();
        if old != canonical {
            let q = Query::update()
                .table(Alias::new(table))
                .value(Alias::new("hlc_uuid"), canonical.clone())
                .and_where(Expr::col(Alias::new("id")).eq(id))
                .to_owned();
            tx.execute(&q).await?;
        }
        out.insert(id, canonical);
    }
    Ok(out)
}
async fn add(tx: &DatabaseTransaction, table: &str, col: &str, kind: &str) -> Result<(), DbErr> {
    let mut d = ColumnDef::new(Alias::new(col));
    match kind {
        "integer" => {
            d.integer();
        }
        "bigint" => {
            d.big_integer();
        }
        "bool" => {
            d.boolean().not_null().default(false);
        }
        _ => {
            d.text();
        }
    };
    SchemaManager::new(tx)
        .alter_table(
            Table::alter()
                .table(Alias::new(table))
                .add_column(d)
                .to_owned(),
        )
        .await
}
async fn update(
    tx: &DatabaseTransaction,
    table: &str,
    id: i32,
    values: Vec<(&str, Value)>,
) -> Result<(), DbErr> {
    let mut q = Query::update();
    q.table(Alias::new(table));
    for (name, value) in values {
        q.value(Alias::new(name), value);
    }
    q.and_where(Expr::col(Alias::new("id")).eq(id));
    tx.execute(&q.to_owned()).await?;
    Ok(())
}
async fn ensure_unpublished(tx: &DatabaseTransaction, family: &str, id: i32) -> Result<(), DbErr> {
    let schema = SchemaManager::new(tx);
    for table in [
        "sync_candidates",
        "sync_journal",
        "sync_checkpoints",
        "sync_objects",
    ] {
        if schema.has_table(table).await? {
            let query = Query::select()
                .expr(Expr::val(1))
                .from(Alias::new(table))
                .limit(1)
                .to_owned();
            if tx.query_one(&query).await?.is_some() {
                return Err(invalid(format!(
                    "{family} row {id}: identity correction requires unpublished state"
                )));
            }
        }
    }
    Ok(())
}
#[derive(Clone)]
struct Column {
    name: String,
    ty: String,
    required: bool,
    default: Option<String>,
    primary: bool,
}
async fn has_sequence(tx: &DatabaseTransaction) -> Result<bool, DbErr> {
    let q = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sqlite_master"))
        .and_where(Expr::col(Alias::new("type")).eq("table"))
        .and_where(Expr::col(Alias::new("name")).eq("sqlite_sequence"))
        .limit(1)
        .to_owned();
    Ok(tx.query_one(&q).await?.is_some())
}
async fn rebuild(
    tx: &DatabaseTransaction,
    table: &str,
    removed: &[&str],
    required: &[&str],
    mapped: &[(&str, &str)],
    skipped_unique: &[&str],
) -> Result<(), DbErr> {
    let schema = SchemaManager::new(tx);
    if !schema.has_table(table).await? {
        return Ok(());
    }
    let triggers = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sqlite_master"))
        .and_where(Expr::col(Alias::new("type")).eq("trigger"))
        .and_where(Expr::col(Alias::new("tbl_name")).eq(table))
        .limit(1)
        .to_owned();
    if tx.query_one(&triggers).await?.is_some() {
        return Err(invalid(format!(
            "{table}: triggers require explicit migration"
        )));
    }
    let info = tx.query_all(&pragma("pragma_table_info", table)).await?;
    let mut columns = Vec::new();
    for r in info {
        let name: String = r.try_get("", "name")?;
        if removed.contains(&name.as_str()) {
            continue;
        }
        let ty: String = r.try_get("", "type")?;
        let required_old: i64 = r.try_get("", "notnull")?;
        let pk: i64 = r.try_get("", "pk")?;
        let default: Option<String> = r.try_get("", "dflt_value")?;
        columns.push(Column {
            name: name.clone(),
            ty,
            required: required_old != 0 || required.contains(&name.as_str()),
            default,
            primary: pk != 0,
        });
    }
    let mut indexes = Vec::<(String, bool, Vec<String>)>::new();
    for index in tx.query_all(&pragma("pragma_index_list", table)).await? {
        let name: String = index.try_get("", "name")?;
        let unique: i64 = index.try_get("", "unique")?;
        let partial: i64 = index.try_get("", "partial")?;
        if partial != 0 {
            return Err(invalid(format!(
                "{table}: partial index {name} cannot be preserved"
            )));
        }
        let mut keys = Vec::new();
        for key in tx.query_all(&pragma("pragma_index_info", &name)).await? {
            let key: Option<String> = key.try_get("", "name")?;
            let key = key.ok_or_else(|| {
                invalid(format!(
                    "{table}: expression index {name} cannot be preserved"
                ))
            })?;
            keys.push(
                mapped
                    .iter()
                    .find(|(old, _)| *old == key)
                    .map_or(key.clone(), |(_, new)| (*new).to_owned()),
            );
        }
        if keys.is_empty()
            || keys.iter().any(|key| removed.contains(&key.as_str()))
            || skipped_unique.contains(&name.as_str())
        {
            continue;
        }
        let final_name = if name.starts_with("sqlite_autoindex_") {
            format!("p2_{table}_{}", indexes.len())
        } else {
            name
        };
        indexes.push((final_name, unique != 0, keys));
    }
    let seq = if has_sequence(tx).await? {
        let q = Query::select()
            .column(Alias::new("seq"))
            .from(Alias::new("sqlite_sequence"))
            .and_where(Expr::col(Alias::new("name")).eq(table))
            .to_owned();
        tx.query_one(&q)
            .await?
            .map(|r| r.try_get::<i64>("", "seq"))
            .transpose()?
    } else {
        None
    };
    let tmp = format!("p2_rebuild_{table}");
    let mut create = Table::create();
    create.table(Alias::new(&tmp));
    for col in &columns {
        let mut def = ColumnDef::new(Alias::new(&col.name));
        if col.ty.is_empty()
            || !col
                .ty
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '(' | ')' | ',' | '_'))
        {
            return Err(invalid(format!(
                "{table}.{} has unsupported SQLite type {:?}",
                col.name, col.ty
            )));
        }
        def.custom(Alias::new(&col.ty));
        if col.primary {
            def.integer().not_null().auto_increment().primary_key();
        } else if col.required {
            def.not_null();
        }
        if let Some(raw) = &col.default {
            if raw.starts_with('\'') && raw.ends_with('\'') {
                def.default(raw[1..raw.len() - 1].replace("''", "'"));
            } else if raw.eq_ignore_ascii_case("true") {
                def.default(true);
            } else if raw.eq_ignore_ascii_case("false") {
                def.default(false);
            } else if let Ok(v) = raw.parse::<i64>() {
                def.default(v);
            } else if let Ok(v) = raw.parse::<f64>() {
                def.default(v);
            } else {
                return Err(invalid(format!(
                    "{table}.{} has unsupported default {raw}",
                    col.name
                )));
            }
        }
        create.col(def);
    }
    schema.create_table(create.to_owned()).await?;
    let names: Vec<Alias> = columns.iter().map(|c| Alias::new(&c.name)).collect();
    let select = Query::select()
        .columns(names.clone())
        .from(Alias::new(table))
        .to_owned();
    let insert = Query::insert()
        .into_table(Alias::new(&tmp))
        .columns(names)
        .select_from(select)
        .map_err(|e| invalid(e.to_string()))?
        .to_owned();
    tx.execute(&insert).await?;
    schema
        .drop_table(Table::drop().table(Alias::new(table)).to_owned())
        .await?;
    schema
        .rename_table(
            Table::rename()
                .table(Alias::new(&tmp), Alias::new(table))
                .to_owned(),
        )
        .await?;
    for (name, unique, keys) in indexes {
        let mut i = Index::create();
        i.name(&name).table(Alias::new(table));
        if unique {
            i.unique();
        }
        for key in keys {
            i.col(Alias::new(key));
        }
        schema.create_index(i.to_owned()).await?;
    }
    if let Some(seq) = seq {
        let exists = Query::select()
            .expr(Expr::val(1))
            .from(Alias::new("sqlite_sequence"))
            .and_where(Expr::col(Alias::new("name")).eq(table))
            .limit(1)
            .to_owned();
        if tx.query_one(&exists).await?.is_none() {
            let insert = Query::insert()
                .into_table(Alias::new("sqlite_sequence"))
                .columns(["name", "seq"].map(Alias::new))
                .values_panic([table.into(), seq.into()])
                .to_owned();
            tx.execute(&insert).await?;
        }
        let q = Query::update()
            .table(Alias::new("sqlite_sequence"))
            .value(Alias::new("seq"), seq)
            .and_where(Expr::col(Alias::new("name")).eq(table))
            .and_where(Expr::col(Alias::new("seq")).lt(seq))
            .to_owned();
        tx.execute(&q).await?;
    }
    Ok(())
}

pub async fn track_sequence(tx: &DatabaseTransaction) -> Result<Option<i64>, DbErr> {
    if !has_sequence(tx).await? {
        return Ok(None);
    }
    let q = Query::select()
        .column(Alias::new("seq"))
        .from(Alias::new("sqlite_sequence"))
        .and_where(Expr::col(Alias::new("name")).eq("media_files"))
        .to_owned();
    tx.query_one(&q)
        .await?
        .map(|r| r.try_get::<i64>("", "seq"))
        .transpose()
}
pub async fn convert(tx: &DatabaseTransaction, initial_sequence: Option<i64>) -> Result<(), DbErr> {
    let schema = SchemaManager::new(tx);
    let mut parents: HashMap<&str, HashMap<i32, String>> = HashMap::new();
    for table in [
        "media_files",
        "artists",
        "genres",
        "albums",
        "playlists",
        "mixes",
    ] {
        parents.insert(table, identities(tx, table).await?);
    }
    for table in [
        "media_file_artists",
        "media_file_genres",
        "media_file_albums",
        "media_file_playlists",
        "media_file_fingerprint",
        "media_file_similarity",
        "mix_queries",
    ] {
        if schema.has_table(table).await? {
            identities(tx, table).await?;
        }
    }
    if schema.has_table("media_metadata").await? {
        add(tx, "media_metadata", "position", "integer").await?;
        let mut positions = BTreeMap::<i32, i32>::new();
        let mut seen = HashSet::new();
        for row in rows(tx, "media_metadata", &["id", "file_id", "hlc_uuid"]).await? {
            let id: i32 = row.try_get("", "id")?;
            let owner: i32 = row.try_get("", "file_id")?;
            let stored: String = row.try_get("", "hlc_uuid")?;
            let mut uuid = if stored.is_empty() {
                Uuid::new_v4()
            } else {
                Uuid::parse_str(&stored)
                    .map_err(|_| invalid(format!("media_metadata row {id}: invalid UUID")))?
            };
            // Multiple source tags can have the same name and an old name-based
            // identity. Each occurrence owns a stable key within its track.
            if uuid.is_nil() || !seen.insert((owner, uuid)) {
                uuid = Uuid::new_v4();
                seen.insert((owner, uuid));
            }
            let position = positions.entry(owner).or_default();
            update(
                tx,
                "media_metadata",
                id,
                vec![
                    ("position", (*position).into()),
                    ("hlc_uuid", uuid.to_string().into()),
                ],
            )
            .await?;
            *position = position
                .checked_add(1)
                .ok_or_else(|| invalid("metadata order overflow"))?;
        }
    }
    let mut artworks = HashMap::new();
    if schema.has_table("media_cover_art").await? {
        for col in ["content_hash", "content_size", "local_available"] {
            add(
                tx,
                "media_cover_art",
                col,
                if col == "content_size" {
                    "bigint"
                } else if col == "local_available" {
                    "bool"
                } else {
                    "text"
                },
            )
            .await?;
        }
        let has_checked_at = schema
            .has_column("media_cover_art", "updated_at_hlc_ts")
            .await?;
        let mut columns = vec!["id", "hlc_uuid", "file_hash", "binary"];
        if has_checked_at {
            columns.push("updated_at_hlc_ts");
        }
        for row in rows(tx, "media_cover_art", &columns).await? {
            let id: i32 = row.try_get("", "id")?;
            let uuid: String = row.try_get("", "hlc_uuid")?;
            let old_hash: String = row.try_get("", "file_hash")?;
            let checked_at: Option<String> = if has_checked_at {
                Some(row.try_get("", "updated_at_hlc_ts")?)
            } else {
                None
            };
            let body: Vec<u8> = row.try_get("", "binary")?;
            let placeholder = body.is_empty()
                && old_hash.is_empty()
                && (uuid.is_empty() || Uuid::parse_str(&uuid).is_ok_and(|v| v.is_nil()));
            if placeholder || body.len() > 8 * 1024 * 1024 {
                update(
                    tx,
                    "media_cover_art",
                    id,
                    vec![("local_available", false.into())],
                )
                .await?;
                artworks.insert(id, (None, checked_at));
                continue;
            }
            let parsed = Uuid::parse_str(&uuid)
                .map_err(|_| invalid(format!("media_cover_art row {id} invalid UUID")))?;
            if parsed.is_nil() || body.is_empty() {
                return Err(invalid(format!(
                    "media_cover_art row {id} invalid body or UUID"
                )));
            }
            let hash = blake3::hash(&body).to_hex().to_string();
            let len = <i64 as TryFrom<usize>>::try_from(body.len())
                .map_err(|_| invalid("artwork size overflow"))?;
            update(
                tx,
                "media_cover_art",
                id,
                vec![
                    ("content_hash", hash.into()),
                    ("content_size", len.into()),
                    ("local_available", true.into()),
                ],
            )
            .await?;
            artworks.insert(id, (Some(parsed.to_string()), checked_at));
        }
    }
    type ReferenceColumn<'a> = (&'a str, &'a str, &'a str);
    let specs: [(&str, &[ReferenceColumn<'_>]); 8] = [
        (
            "media_file_artists",
            &[
                ("media_file_id", "track_uuid", "media_files"),
                ("artist_id", "artist_uuid", "artists"),
            ],
        ),
        (
            "media_file_genres",
            &[
                ("media_file_id", "track_uuid", "media_files"),
                ("genre_id", "genre_uuid", "genres"),
            ],
        ),
        (
            "media_file_albums",
            &[
                ("media_file_id", "track_uuid", "media_files"),
                ("album_id", "album_uuid", "albums"),
            ],
        ),
        (
            "media_file_playlists",
            &[
                ("playlist_id", "playlist_uuid", "playlists"),
                ("media_file_id", "track_uuid", "media_files"),
            ],
        ),
        (
            "media_file_likes",
            &[("media_file_id", "track_uuid", "media_files")],
        ),
        (
            "media_file_stats",
            &[("media_file_id", "track_uuid", "media_files")],
        ),
        (
            "media_file_fingerprint",
            &[("media_file_id", "track_uuid", "media_files")],
        ),
        (
            "media_file_similarity",
            &[
                ("file_id1", "first_track_uuid", "media_files"),
                ("file_id2", "second_track_uuid", "media_files"),
            ],
        ),
    ];
    for (table, refs) in specs {
        if !schema.has_table(table).await? {
            continue;
        }
        for (_, new, _) in refs {
            add(tx, table, new, "text").await?;
        }
        let mut cols = vec!["id"];
        if table == "media_file_albums" {
            cols.push("hlc_uuid");
        }
        cols.extend(refs.iter().map(|(old, _, _)| *old));
        let data = rows(tx, table, &cols).await?;
        let mut pairs = HashSet::new();
        for row in data {
            let id: i32 = row.try_get("", "id")?;
            let mut values = Vec::new();
            let mut keys = Vec::new();
            for (old, new, parent) in refs {
                let old_id: i32 = row.try_get("", old)?;
                let uuid = parents
                    .get(parent)
                    .and_then(|p| p.get(&old_id))
                    .ok_or_else(|| invalid(format!("{table} row {id}: missing {parent} {old_id}")))?
                    .clone();
                keys.push(uuid.clone());
                values.push((*new, uuid.into()));
            }
            if matches!(
                table,
                "media_file_artists" | "media_file_genres" | "media_file_albums"
            ) && !pairs.insert(keys.clone())
            {
                return Err(invalid(format!("{table} row {id}: duplicate relationship")));
            }
            if table == "media_file_albums" {
                let previous: String = row.try_get("", "hlc_uuid")?;
                if previous != keys[0] {
                    ensure_unpublished(tx, table, id).await?;
                }
                values.push(("hlc_uuid", keys[0].clone().into()));
            }
            update(tx, table, id, values).await?;
        }
    }
    if schema.has_table("mix_queries").await? {
        for (name, kind) in [
            ("mix_uuid", "text"),
            ("position", "integer"),
            ("target_uuid", "text"),
            ("local_directory", "text"),
        ] {
            add(tx, "mix_queries", name, kind).await?;
        }
        let mut order = BTreeMap::<i32, i32>::new();
        for row in rows(
            tx,
            "mix_queries",
            &["id", "mix_id", "operator", "parameter"],
        )
        .await?
        {
            let id: i32 = row.try_get("", "id")?;
            let mix: i32 = row.try_get("", "mix_id")?;
            let operator: String = row.try_get("", "operator")?;
            let parameter: String = row.try_get("", "parameter")?;
            let owner = parents
                .get("mixes")
                .and_then(|m| m.get(&mix))
                .ok_or_else(|| invalid(format!("mix_queries row {id}: missing mix {mix}")))?
                .clone();
            let family = match operator.as_str() {
                "lib::artist" => Some("artists"),
                "lib::genre" => Some("genres"),
                "lib::album" => Some("albums"),
                "lib::playlist" => Some("playlists"),
                "lib::track" => Some("media_files"),
                _ => None,
            };
            let mut values = vec![("mix_uuid", owner.into())];
            if let Some(family) = family {
                let target = parameter
                    .parse::<i32>()
                    .ok()
                    .and_then(|old_id| parents.get(family).and_then(|p| p.get(&old_id)));
                let Some(target) = target else {
                    eprintln!("Dropping mix {mix} clause {id}: {operator} {parameter}");
                    tx.execute(
                        &Query::delete()
                            .from_table(Alias::new("mix_queries"))
                            .and_where(Expr::col(Alias::new("id")).eq(id))
                            .to_owned(),
                    )
                    .await?;
                    continue;
                };
                values.push(("target_uuid", target.clone().into()));
                values.push(("parameter", String::new().into()));
            } else if matches!(
                operator.as_str(),
                "lib::directory.deep" | "lib::directory.shallow"
            ) {
                values.push(("local_directory", parameter.clone().into()));
                values.push(("parameter", String::new().into()));
            } else {
                let valid = match operator.as_str() {
                    "lib::all"
                    | "lib::queue"
                    | "sort::track_number"
                    | "sort::last_modified"
                    | "sort::duration"
                    | "sort::playedthrough"
                    | "sort::skipped"
                    | "filter::liked"
                    | "filter::analyzed"
                    | "filter::with_cover_art" => parameter.parse::<bool>().is_ok(),
                    "lib::random" | "pipe::recommend" => parameter.parse::<i32>().is_ok(),
                    "pipe::limit" => parameter.parse::<u64>().is_ok(),
                    _ => false,
                };
                if !valid {
                    eprintln!("Dropping mix {mix} clause {id}: {operator} {parameter}");
                    tx.execute(
                        &Query::delete()
                            .from_table(Alias::new("mix_queries"))
                            .and_where(Expr::col(Alias::new("id")).eq(id))
                            .to_owned(),
                    )
                    .await?;
                    continue;
                }
            }
            let pos = order.entry(mix).or_insert(0);
            values.push(("position", (*pos).into()));
            *pos = pos
                .checked_add(1)
                .ok_or_else(|| invalid("mix clause overflow"))?;
            update(tx, "mix_queries", id, values).await?;
        }
    }
    if schema.has_table("media_files").await?
        && schema.has_column("media_files", "cover_art_id").await?
    {
        add(tx, "media_files", "cover_art_uuid", "text").await?;
        add(tx, "media_files", "cover_art_checked_at", "text").await?;
        for row in rows(tx, "media_files", &["id", "cover_art_id"]).await? {
            let id: i32 = row.try_get("", "id")?;
            let old: Option<i32> = row.try_get("", "cover_art_id")?;
            if let Some(old) = old {
                let (uuid, checked_at) = artworks.get(&old).ok_or_else(|| {
                    invalid(format!("media_files row {id}: missing artwork {old}"))
                })?;
                update(
                    tx,
                    "media_files",
                    id,
                    vec![
                        ("cover_art_uuid", uuid.clone().into()),
                        ("cover_art_checked_at", checked_at.clone().into()),
                    ],
                )
                .await?;
            }
        }
    }
    for (table, refs) in specs {
        let removed: Vec<&str> = refs.iter().map(|(old, _, _)| *old).collect();
        let required: Vec<&str> = refs.iter().map(|(_, new, _)| *new).collect();
        let mapped: Vec<(&str, &str)> = refs.iter().map(|(old, new, _)| (*old, *new)).collect();
        let skip = if table == "media_file_similarity" {
            vec!["idx_file_ids", "sqlite_autoindex_media_file_similarity_1"]
        } else {
            vec![]
        };
        rebuild(tx, table, &removed, &required, &mapped, &skip).await?;
    }
    if schema.has_table("mix_queries").await? {
        rebuild(
            tx,
            "mix_queries",
            &["mix_id"],
            &["mix_uuid", "position"],
            &[("mix_id", "mix_uuid")],
            &[],
        )
        .await?;
    }
    rebuild(tx, "media_metadata", &[], &["position"], &[], &[]).await?;
    for table in ["media_analysis", "playback_queue"] {
        rebuild(tx, table, &[], &[], &[], &[]).await?;
    }
    if schema.has_table("media_files").await?
        && schema.has_column("media_files", "cover_art_id").await?
    {
        rebuild(
            tx,
            "media_files",
            &["cover_art_id"],
            &[],
            &[("cover_art_id", "cover_art_uuid")],
            &[],
        )
        .await?;
    }
    // Existing local rows may reference tracks removed before this conversion. Keep
    // the new allocation sequence above every retained local reference so those
    // rows cannot attach to an unrelated future track.
    if schema.has_table("media_files").await? && has_sequence(tx).await? {
        let mut highest = initial_sequence.unwrap_or(0);
        for (table, column) in [
            ("media_metadata", "file_id"),
            ("media_analysis", "file_id"),
            ("playback_queue", "media_file_id"),
        ] {
            if schema.has_table(table).await? {
                let query = Query::select()
                    .expr(Expr::col(Alias::new(column)).max())
                    .from(Alias::new(table))
                    .to_owned();
                if let Some(row) = tx.query_one(&query).await? {
                    highest = std::cmp::max(
                        highest,
                        row.try_get_by_index::<Option<i64>>(0)?.unwrap_or(0),
                    );
                }
            }
        }
        let existing = Query::select()
            .expr(Expr::val(1))
            .from(Alias::new("sqlite_sequence"))
            .and_where(Expr::col(Alias::new("name")).eq("media_files"))
            .limit(1)
            .to_owned();
        if highest > 0 && tx.query_one(&existing).await?.is_none() {
            let insert = Query::insert()
                .into_table(Alias::new("sqlite_sequence"))
                .columns(["name", "seq"].map(Alias::new))
                .values_panic(["media_files".into(), highest.into()])
                .to_owned();
            tx.execute(&insert).await?;
        }
        let update = Query::update()
            .table(Alias::new("sqlite_sequence"))
            .value(Alias::new("seq"), highest)
            .and_where(Expr::col(Alias::new("name")).eq("media_files"))
            .and_where(Expr::col(Alias::new("seq")).lt(highest))
            .to_owned();
        tx.execute(&update).await?;
    }
    let indexes: &[(&str, &str, &[&str], bool)] = &[
        ("p2_track_uuid", "media_files", &["hlc_uuid"], true),
        ("p2_artist_uuid", "artists", &["hlc_uuid"], true),
        ("p2_genre_uuid", "genres", &["hlc_uuid"], true),
        ("p2_album_uuid", "albums", &["hlc_uuid"], true),
        ("p2_playlist_uuid", "playlists", &["hlc_uuid"], true),
        ("p2_mix_uuid", "mixes", &["hlc_uuid"], true),
        ("p2_artwork_uuid", "media_cover_art", &["hlc_uuid"], true),
        (
            "p2_artist_pair",
            "media_file_artists",
            &["track_uuid", "artist_uuid"],
            true,
        ),
        (
            "p2_artist_reverse",
            "media_file_artists",
            &["artist_uuid", "track_uuid"],
            false,
        ),
        (
            "p2_genre_pair",
            "media_file_genres",
            &["track_uuid", "genre_uuid"],
            true,
        ),
        (
            "p2_genre_reverse",
            "media_file_genres",
            &["genre_uuid", "track_uuid"],
            false,
        ),
        ("p2_album_track", "media_file_albums", &["track_uuid"], true),
        (
            "p2_album_reverse",
            "media_file_albums",
            &["album_uuid", "track_uuid"],
            false,
        ),
        (
            "p2_playlist_order",
            "media_file_playlists",
            &["playlist_uuid", "position", "hlc_uuid"],
            false,
        ),
        (
            "p2_fingerprint_track",
            "media_file_fingerprint",
            &["track_uuid"],
            false,
        ),
        (
            "p2_mix_order",
            "mix_queries",
            &["mix_uuid", "position"],
            true,
        ),
        (
            "p2_playlist_track",
            "media_file_playlists",
            &["track_uuid", "playlist_uuid"],
            false,
        ),
        (
            "p2_stats_owner",
            "media_file_stats",
            &["track_uuid", "device_id"],
            true,
        ),
        ("p2_like_track", "media_file_likes", &["track_uuid"], true),
        (
            "p2_similarity_first",
            "media_file_similarity",
            &["first_track_uuid", "second_track_uuid"],
            false,
        ),
        (
            "p2_similarity_second",
            "media_file_similarity",
            &["second_track_uuid", "first_track_uuid"],
            false,
        ),
        ("p2_mix_target", "mix_queries", &["target_uuid"], false),
        (
            "p2_metadata_order",
            "media_metadata",
            &["file_id", "position"],
            true,
        ),
        (
            "p2_metadata_occurrence",
            "media_metadata",
            &["file_id", "hlc_uuid"],
            true,
        ),
        (
            "p2_track_artwork",
            "media_files",
            &["cover_art_uuid"],
            false,
        ),
    ];
    for &(name, table, cols, unique) in indexes {
        if !schema.has_table(table).await? {
            continue;
        }
        let mut missing = false;
        for column in cols {
            if !schema.has_column(table, column).await? {
                missing = true;
                break;
            }
        }
        if missing {
            continue;
        }
        let mut q = Index::create();
        q.name(name).table(Alias::new(table)).if_not_exists();
        if unique {
            q.unique();
        }
        for col in cols {
            q.col(Alias::new(*col));
        }
        schema.create_index(q.to_owned()).await?;
    }
    for table in [
        "media_file_artists",
        "media_file_genres",
        "media_file_albums",
        "media_file_playlists",
        "media_file_likes",
        "media_file_stats",
        "media_file_fingerprint",
        "media_file_similarity",
        "mix_queries",
    ] {
        if schema.has_table(table).await? {
            let name = format!("p2_{table}_uuid");
            schema
                .create_index(
                    Index::create()
                        .name(&name)
                        .table(Alias::new(table))
                        .col(Alias::new("hlc_uuid"))
                        .unique()
                        .if_not_exists()
                        .to_owned(),
                )
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm_migration::{
        MigrationTrait, MigratorTrait,
        sea_orm::{Database, Statement},
    };
    async fn before_conversion() -> sea_orm_migration::sea_orm::DatabaseConnection {
        let _ = crate::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned());
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("PRAGMA foreign_keys=ON")
            .await
            .unwrap();
        let position = crate::Migrator::migrations()
            .iter()
            .position(|migration| migration.name() == "m20261101_000032_create_sync_library")
            .unwrap();
        crate::Migrator::up(&db, Some(position as u32))
            .await
            .unwrap();
        db
    }
    #[tokio::test]
    async fn legacy_duplicate_identities_are_regenerated() {
        let _ = crate::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned());
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("PRAGMA foreign_keys=ON")
            .await
            .unwrap();
        let position = crate::Migrator::migrations()
            .iter()
            .position(|m| m.name() == "m20261101_000031_create_sync_store")
            .unwrap();
        crate::Migrator::up(&db, Some(position as u32))
            .await
            .unwrap();
        for sql in [
            "INSERT INTO media_files(id,file_name,directory,extension,file_hash,last_modified,sample_rate,duration,hlc_uuid) VALUES(7,'a','','ogg','crc','2020',44100,1,'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'),(8,'b','','ogg','crc','2020',44100,1,'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')",
            "INSERT INTO media_file_stats(id,media_file_id,liked,skipped,played_through,updated_at) VALUES(17,7,1,3,9,'2020'),(18,8,0,4,10,'2020')",
            "INSERT INTO artists(id,name,\"group\") VALUES(3,'Artist','g')",
            "INSERT INTO media_file_artists(id,media_file_id,artist_id) VALUES(10,7,3),(11,8,3)",
            "INSERT INTO playlists(id,name,\"group\",hlc_uuid) VALUES(6,'Playlist','g','cccccccc-cccc-4ccc-8ccc-cccccccccccc')",
            "INSERT INTO media_file_playlists(id,playlist_id,media_file_id,position,hlc_uuid) VALUES(14,6,7,0,'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee'),(15,6,7,1,'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee')",
            "INSERT INTO mixes(id,name,\"group\",locked,scriptlet_mode,hlc_uuid) VALUES(999,'Mix','g',0,0,'dddddddd-dddd-4ddd-8ddd-dddddddddddd')",
            "INSERT INTO mix_queries(id,mix_id,operator,parameter,\"group\",hlc_uuid) VALUES(1001,999,'lib::all','true',0,'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'),(1002,999,'lib::all','true',0,'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'),(1003,999,'lib::all','true',0,'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa')",
        ] {
            db.execute_unprepared(sql).await.unwrap();
        }
        crate::up_with_actor(&db, Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"), None)
            .await
            .unwrap();
        for (table, ids, original) in [
            ("media_files", "7,8", "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"),
            (
                "media_file_playlists",
                "14,15",
                "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee",
            ),
            (
                "mix_queries",
                "1001,1002,1003",
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            ),
        ] {
            let rows = db
                .query_all_raw(Statement::from_string(
                    db.get_database_backend(),
                    format!("SELECT hlc_uuid FROM {table} WHERE id IN ({ids}) ORDER BY id"),
                ))
                .await
                .unwrap();
            let mut seen = HashSet::new();
            for (i, row) in rows.iter().enumerate() {
                let value: String = row.try_get("", "hlc_uuid").unwrap();
                let uuid = Uuid::parse_str(&value).unwrap();
                assert!(!uuid.is_nil() && value == uuid.to_string() && seen.insert(uuid));
                if i == 0 {
                    assert_eq!(value, original);
                }
            }
        }
        let rows = db.query_all_raw(Statement::from_string(db.get_database_backend(), "SELECT f.hlc_uuid AS file_uuid,s.hlc_uuid AS stat_uuid,s.track_uuid,s.device_id,s.skipped,s.played_through,l.track_uuid AS liked_track FROM media_file_stats s JOIN media_files f ON f.hlc_uuid=s.track_uuid LEFT JOIN media_file_likes l ON l.track_uuid=f.hlc_uuid ORDER BY f.id".to_owned())).await.unwrap();
        assert_eq!(rows.len(), 2);
        for (i, row) in rows.iter().enumerate() {
            let file: String = row.try_get("", "file_uuid").unwrap();
            let device: String = row.try_get("", "device_id").unwrap();
            assert_eq!(
                row.try_get::<String>("", "stat_uuid").unwrap(),
                Uuid::new_v5(
                    &Uuid::parse_str(&file).unwrap(),
                    Uuid::parse_str(&device).unwrap().as_bytes()
                )
                .to_string()
            );
            assert_eq!(
                row.try_get::<Option<String>>("", "liked_track").unwrap(),
                if i == 0 { Some(file) } else { None }
            );
            assert_eq!(
                row.try_get::<i32>("", "played_through").unwrap(),
                9 + i as i32
            );
            assert_eq!(row.try_get::<i32>("", "skipped").unwrap(), 3 + i as i32);
        }
    }
    #[tokio::test]
    async fn legacy_broken_mix_clauses_are_dropped_per_clause() {
        let db = before_conversion().await;
        for sql in [
            "INSERT INTO artists(id,name,\"group\") VALUES(3,'Artist','g')",
            "INSERT INTO mixes(id,name,\"group\",locked,scriptlet_mode,hlc_uuid) VALUES(999,'Mix','g',0,0,'dddddddd-dddd-4ddd-8ddd-dddddddddddd'),(998,'Other','g',0,0,'cccccccc-cccc-4ccc-8ccc-cccccccccccc')",
            "INSERT INTO mix_queries(id,mix_id,operator,parameter,\"group\",hlc_uuid) VALUES(1001,999,'lib::artist','3',0,'11111111-1111-4111-8111-111111111111'),(1002,999,'lib::track','9999',0,'22222222-2222-4222-8222-222222222222'),(1003,999,'lib::unknown','x',0,'33333333-3333-4333-8333-333333333333'),(1004,999,'pipe::limit','abc',0,'44444444-4444-4444-8444-444444444444'),(1005,998,'lib::all','true',0,'55555555-5555-4555-8555-555555555555')",
        ] {
            db.execute_unprepared(sql).await.unwrap();
        }
        crate::up_with_actor(&db, None, None).await.unwrap();
        let rows = db
            .query_all_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT id,position FROM mix_queries WHERE id BETWEEN 1001 AND 1005 ORDER BY id"
                    .to_owned(),
            ))
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].try_get::<i32>("", "id").unwrap(), 1001);
        assert_eq!(rows[0].try_get::<i32>("", "position").unwrap(), 0);
        assert_eq!(rows[1].try_get::<i32>("", "id").unwrap(), 1005);
    }
    #[tokio::test]
    async fn oversized_legacy_artwork_is_kept_unavailable() {
        let db = before_conversion().await;
        db.execute_unprepared("INSERT INTO media_cover_art(id,file_hash,binary,primary_color,hlc_uuid) VALUES(11,'old',zeroblob(8388609),17,'')").await.unwrap();
        db.execute_unprepared("INSERT INTO media_files(id,file_name,directory,extension,file_hash,last_modified,cover_art_id,sample_rate,duration,hlc_uuid) VALUES(7,'a','','ogg','crc','2020',11,44100,1,'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')").await.unwrap();
        crate::up_with_actor(&db, None, None).await.unwrap();
        let row = db.query_one_raw(Statement::from_string(db.get_database_backend(), "SELECT a.local_available,length(a.binary) AS size,f.cover_art_uuid FROM media_cover_art a JOIN media_files f ON f.id=7 WHERE a.id=11".to_owned())).await.unwrap().unwrap();
        assert!(!row.try_get::<bool>("", "local_available").unwrap());
        assert_eq!(row.try_get::<i32>("", "size").unwrap(), 8388609);
        assert_eq!(
            row.try_get::<Option<String>>("", "cover_art_uuid").unwrap(),
            None
        );
    }
    #[tokio::test]
    async fn populated_conversion_preserves_rows_and_removes_fanout() {
        let db = before_conversion().await;
        for statement in [
            "INSERT INTO media_cover_art(id,file_hash,binary,primary_color,hlc_uuid) VALUES(11,'old',x'010203',17,'')",
            "INSERT INTO media_cover_art(id,file_hash,binary,primary_color,hlc_uuid) VALUES(12,'',x'',0,'00000000-0000-0000-0000-000000000000')",
            "INSERT INTO media_files(id,file_name,directory,extension,file_hash,last_modified,cover_art_id,sample_rate,duration,hlc_uuid) VALUES(8,'empty','/music','ogg','other','2020',12,44100,12,'33333333-3333-4333-8333-333333333333')",
            "INSERT INTO media_files(id,file_name,directory,extension,file_hash,last_modified,cover_art_id,sample_rate,duration,hlc_uuid) VALUES(7,'a','/music','ogg','crc','2020',11,44100,12,'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')",
            "INSERT INTO media_files(id,file_name,directory,extension,file_hash,last_modified,sample_rate,duration,hlc_uuid) VALUES(600,'removed','/music','ogg','gone','2020',44100,1,'44444444-4444-4444-8444-444444444444')",
            "DELETE FROM media_files WHERE id=600",
            "INSERT INTO artists(id,name,\"group\") VALUES(3,'Artist','g')",
            "INSERT INTO genres(id,name,\"group\") VALUES(4,'Genre','g')",
            "INSERT INTO albums(id,name,\"group\") VALUES(5,'Album','g')",
            "INSERT INTO playlists(id,name,\"group\",hlc_uuid) VALUES(6,'Playlist','g','cccccccc-cccc-4ccc-8ccc-cccccccccccc')",
            "INSERT INTO mixes(id,name,\"group\",locked,scriptlet_mode,hlc_uuid) VALUES(999,'Mix','g',0,0,'dddddddd-dddd-4ddd-8ddd-dddddddddddd')",
            "INSERT INTO media_file_artists(id,media_file_id,artist_id) VALUES(10,7,3)",
            "INSERT INTO media_file_genres(id,media_file_id,genre_id) VALUES(12,7,4)",
            "INSERT INTO media_file_albums(id,media_file_id,album_id,track_number) VALUES(13,7,5,2)",
            "INSERT INTO media_file_playlists(id,playlist_id,media_file_id,position,hlc_uuid) VALUES(14,6,7,0,'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee'),(15,6,7,1,'ffffffff-ffff-4fff-8fff-ffffffffffff')",
            "INSERT INTO mix_queries(id,mix_id,operator,parameter,\"group\",hlc_uuid) VALUES(1001,999,'lib::genre','4',0,'11111111-1111-4111-8111-111111111111'),(1002,999,'lib::directory.deep','/local',0,'22222222-2222-4222-8222-222222222222')",
            "INSERT INTO media_file_stats(id,media_file_id,liked,skipped,played_through,updated_at) VALUES(18,7,1,3,9,'2020')",
            "INSERT INTO media_metadata(id,file_id,meta_key,meta_value,hlc_uuid) VALUES(19,7,'artist','First','66666666-6666-4666-8666-666666666666'),(22,7,'artist','Second','66666666-6666-4666-8666-666666666666')",
            "INSERT INTO media_analysis(id,file_id) VALUES(20,7)",
            "INSERT INTO playback_queue(id,media_file_id) VALUES(21,7)",
        ] {
            db.execute_unprepared(statement)
                .await
                .unwrap_or_else(|error| panic!("{statement}: {error}"));
        }
        db.execute_unprepared("PRAGMA foreign_keys=OFF")
            .await
            .unwrap();
        db.execute_unprepared("INSERT INTO playback_queue(id,media_file_id) VALUES(22,500)")
            .await
            .unwrap();
        db.execute_unprepared("PRAGMA foreign_keys=ON")
            .await
            .unwrap();
        crate::with_migration_actor(
            Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            super::super::Migration.up(&SchemaManager::new(&db)),
        )
        .await
        .unwrap();
        let row=db.query_one_raw(Statement::from_string(db.get_database_backend(),"SELECT m.id,m.directory,m.file_name,m.cover_art_uuid,a.hlc_uuid AS artwork_uuid,a.content_hash,a.content_size,a.local_available FROM media_files m JOIN media_cover_art a ON a.hlc_uuid=m.cover_art_uuid".to_owned())).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i32>("", "id").unwrap(), 7);
        assert_eq!(row.try_get::<String>("", "directory").unwrap(), "/music");
        assert_eq!(
            row.try_get::<String>("", "content_hash").unwrap(),
            blake3::hash(&[1, 2, 3]).to_hex().to_string()
        );
        assert_eq!(row.try_get::<i64>("", "content_size").unwrap(), 3);
        assert!(row.try_get::<bool>("", "local_available").unwrap());
        let empty = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT cover_art_uuid,cover_art_checked_at FROM media_files WHERE id=8".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            empty
                .try_get::<Option<String>>("", "cover_art_uuid")
                .unwrap(),
            None
        );
        assert_eq!(
            empty
                .try_get::<Option<String>>("", "cover_art_checked_at")
                .unwrap()
                .as_deref(),
            None
        );
        let rows = db
            .query_all_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT id,playlist_uuid,track_uuid,position FROM media_file_playlists ORDER BY id"
                    .to_owned(),
            ))
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].try_get::<i32>("", "id").unwrap(), 14);
        assert_eq!(rows[1].try_get::<i32>("", "id").unwrap(), 15);
        let mix=db.query_one_raw(Statement::from_string(db.get_database_backend(),"SELECT q.id,q.mix_uuid,q.target_uuid,q.parameter,g.hlc_uuid FROM mix_queries q JOIN genres g ON g.hlc_uuid=q.target_uuid WHERE q.id=1001".to_owned())).await.unwrap().unwrap();
        assert_eq!(mix.try_get::<String>("", "parameter").unwrap(), "");
        let tags = db
            .query_all(
                &Query::select()
                    .columns(
                        ["id", "meta_key", "meta_value", "hlc_uuid", "position"].map(Alias::new),
                    )
                    .from(Alias::new("media_metadata"))
                    .and_where(Expr::col(Alias::new("file_id")).eq(7))
                    .order_by(Alias::new("position"), Order::Asc)
                    .to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].try_get::<i32>("", "id").unwrap(), 19);
        assert_eq!(tags[1].try_get::<i32>("", "id").unwrap(), 22);
        assert_eq!(
            tags[0].try_get::<String>("", "meta_value").unwrap(),
            "First"
        );
        assert_eq!(
            tags[1].try_get::<String>("", "meta_value").unwrap(),
            "Second"
        );
        assert_ne!(
            tags[0].try_get::<String>("", "hlc_uuid").unwrap(),
            tags[1].try_get::<String>("", "hlc_uuid").unwrap()
        );
        assert_eq!(tags[0].try_get::<i32>("", "position").unwrap(), 0);
        assert_eq!(tags[1].try_get::<i32>("", "position").unwrap(), 1);
        let directory = db
            .query_one(
                &Query::select()
                    .columns(["parameter", "local_directory"].map(Alias::new))
                    .from(Alias::new("mix_queries"))
                    .and_where(Expr::col(Alias::new("id")).eq(1002))
                    .to_owned(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(directory.try_get::<String>("", "parameter").unwrap(), "");
        assert_eq!(
            directory.try_get::<String>("", "local_directory").unwrap(),
            "/local"
        );
        assert_eq!(
            mix.try_get::<String>("", "target_uuid").unwrap(),
            mix.try_get::<String>("", "hlc_uuid").unwrap()
        );
        db.execute_unprepared("DELETE FROM media_files WHERE id=7")
            .await
            .unwrap();
        for table in [
            "media_file_artists",
            "media_file_genres",
            "media_file_albums",
            "media_file_playlists",
            "media_file_stats",
            "media_file_likes",
            "media_metadata",
            "media_analysis",
            "playback_queue",
        ] {
            let q = Query::select()
                .expr(Func::count(Expr::val(1)))
                .from(Alias::new(table))
                .to_owned();
            let count: i64 = db
                .query_one(&q)
                .await
                .unwrap()
                .unwrap()
                .try_get_by_index(0)
                .unwrap();
            assert!(count > 0, "{table} was cascaded");
        }
        db.execute_unprepared("INSERT INTO media_files(file_name,directory,extension,file_hash,last_modified,sample_rate,duration,hlc_uuid) VALUES('new','/music','ogg','new','2020',44100,1,'55555555-5555-4555-8555-555555555555')").await.unwrap();
        let row = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT id FROM media_files WHERE file_name='new'".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert!(
            row.try_get::<i32>("", "id").unwrap() > 600,
            "new id {}",
            row.try_get::<i32>("", "id").unwrap()
        );
    }
    #[tokio::test]
    async fn missing_parent_refuses_and_rolls_back_all_schema_changes() {
        let db = before_conversion().await;
        db.execute_unprepared("PRAGMA foreign_keys=OFF")
            .await
            .unwrap();
        db.execute_unprepared(
            "INSERT INTO media_file_genres(id,media_file_id,genre_id) VALUES(9,999,888)",
        )
        .await
        .unwrap();
        assert!(
            super::super::Migration
                .up(&SchemaManager::new(&db))
                .await
                .is_err()
        );
        let schema = SchemaManager::new(&db);
        assert!(
            !schema
                .has_column("media_file_genres", "track_uuid")
                .await
                .unwrap()
        );
        let q = Query::select()
            .expr(Func::count(Expr::val(1)))
            .from(Alias::new("media_file_genres"))
            .to_owned();
        assert_eq!(
            db.query_one(&q)
                .await
                .unwrap()
                .unwrap()
                .try_get_by_index::<i64>(0)
                .unwrap(),
            1
        );
    }
}
