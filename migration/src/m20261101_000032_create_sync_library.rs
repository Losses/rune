use sea_orm_migration::{
    prelude::*,
    sea_orm::{ConnectionTrait, FromQueryResult, TransactionTrait, prelude::Uuid},
};
use std::collections::HashSet;

#[path = "source_uuid_conversion.rs"]
mod source_uuid_conversion;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(FromQueryResult)]
struct LegacyStat {
    id: i32,
    media_file_id: i32,
    liked: bool,
    skipped: i32,
    played_through: i32,
    updated_at: String,
    file_uuid: Option<String>,
}

#[derive(FromQueryResult)]
struct ParentIdentityRow {
    id: i32,
    name: String,
    hlc_uuid: String,
}
#[derive(FromQueryResult)]
struct FingerprintIdentityRow {
    id: i32,
    media_file_id: i32,
    hlc_uuid: String,
    file_uuid: Option<String>,
}
#[derive(FromQueryResult)]
struct SimilarityIdentityRow {
    id: i32,
    file_id1: i32,
    file_id2: i32,
    hlc_uuid: String,
    first_uuid: Option<String>,
    second_uuid: Option<String>,
    similarity: f32,
}
#[derive(FromQueryResult)]
struct ArtworkIdentityRow {
    id: i32,
    file_hash: String,
    binary: Vec<u8>,
    hlc_uuid: String,
}
#[derive(FromQueryResult)]
struct MembershipIdentityRow {
    id: i32,
    media_file_id: i32,
    parent_id: i32,
    hlc_uuid: String,
    file_uuid: Option<String>,
    parent_name: Option<String>,
}

async fn correct_catalog_identities(
    tx: &sea_orm_migration::sea_orm::DatabaseTransaction,
) -> Result<(), DbErr> {
    use crate::catalog_identity::{self, ParentKind};
    let schema = SchemaManager::new(tx);
    let backend = tx.get_database_backend();
    let mut changes: Vec<(&str, i32, String)> = Vec::new();
    for (table, kind) in [
        ("artists", ParentKind::Artist),
        ("genres", ParentKind::Genre),
        ("albums", ParentKind::Album),
    ] {
        if !schema.has_table(table).await? {
            continue;
        }
        let select = Query::select()
            .from(Alias::new(table))
            .columns(["id", "name", "hlc_uuid"].map(Alias::new))
            .to_owned();
        let rows = ParentIdentityRow::find_by_statement(backend.build(&select))
            .all(tx)
            .await?;
        let mut ids = HashSet::new();
        for row in rows {
            let id = catalog_identity::parent(kind, &row.name).to_string();
            if !ids.insert(id.clone()) {
                return Err(DbErr::Custom(format!(
                    "ambiguous {table} canonical identity for row {}",
                    row.id
                )));
            }
            if row.hlc_uuid != id {
                changes.push((table, row.id, id));
            }
        }
    }
    for (table, parent_table, parent_column, kind) in [
        (
            "media_file_artists",
            "artists",
            "artist_id",
            ParentKind::Artist,
        ),
        ("media_file_genres", "genres", "genre_id", ParentKind::Genre),
    ] {
        if !schema.has_table(table).await? {
            continue;
        }
        let select = Query::select()
            .from_as(Alias::new(table), Alias::new("l"))
            .columns(["id", "media_file_id", "hlc_uuid"].map(|v| (Alias::new("l"), Alias::new(v))))
            .expr_as(
                Expr::col((Alias::new("l"), Alias::new(parent_column))),
                Alias::new("parent_id"),
            )
            .expr_as(
                Expr::col((Alias::new("f"), Alias::new("hlc_uuid"))),
                Alias::new("file_uuid"),
            )
            .expr_as(
                Expr::col((Alias::new("p"), Alias::new("name"))),
                Alias::new("parent_name"),
            )
            .join_as(
                JoinType::LeftJoin,
                Alias::new("media_files"),
                Alias::new("f"),
                Expr::col((Alias::new("l"), Alias::new("media_file_id")))
                    .equals((Alias::new("f"), Alias::new("id"))),
            )
            .join_as(
                JoinType::LeftJoin,
                Alias::new(parent_table),
                Alias::new("p"),
                Expr::col((Alias::new("l"), Alias::new(parent_column)))
                    .equals((Alias::new("p"), Alias::new("id"))),
            )
            .to_owned();
        let rows = MembershipIdentityRow::find_by_statement(backend.build(&select))
            .all(tx)
            .await?;
        let mut pairs = HashSet::new();
        let mut ids = HashSet::new();
        for row in rows {
            if !pairs.insert((row.media_file_id, row.parent_id)) {
                return Err(DbErr::Custom(format!(
                    "duplicate {table} pair ({}, {}) at row {}",
                    row.media_file_id, row.parent_id, row.id
                )));
            }
            let file = row
                .file_uuid
                .as_deref()
                .ok_or_else(|| DbErr::Custom(format!("{table} row {} missing file", row.id)))?;
            let file = Uuid::parse_str(file).map_err(|e| {
                DbErr::Custom(format!("{table} row {} invalid file UUID: {e}", row.id))
            })?;
            let parent_name = row
                .parent_name
                .ok_or_else(|| DbErr::Custom(format!("{table} row {} missing parent", row.id)))?;
            let id = catalog_identity::membership(
                file,
                kind,
                catalog_identity::parent(kind, &parent_name),
            )
            .to_string();
            if !ids.insert(id.clone()) {
                return Err(DbErr::Custom(format!(
                    "ambiguous {table} identity at row {}",
                    row.id
                )));
            }
            if row.hlc_uuid != id {
                changes.push((table, row.id, id));
            }
        }
    }
    if schema.has_table("media_cover_art").await? {
        let mut identities = HashSet::new();
        let mut after = 0;
        loop {
            let query = Query::select()
                .from(Alias::new("media_cover_art"))
                .columns(["id", "file_hash", "binary", "hlc_uuid"].map(Alias::new))
                .and_where(Expr::col(Alias::new("id")).gt(after))
                .order_by(Alias::new("id"), Order::Asc)
                .limit(1)
                .to_owned();
            let Some(row) = ArtworkIdentityRow::find_by_statement(backend.build(&query))
                .one(tx)
                .await?
            else {
                break;
            };
            after = row.id;
            if row.binary.is_empty()
                && row.file_hash.is_empty()
                && (row.hlc_uuid.is_empty() || row.hlc_uuid == Uuid::nil().to_string())
            {
                continue;
            }
            if row.binary.len() > 8 * 1024 * 1024 {
                eprintln!(
                    "Artwork row {} is too large and will be unavailable",
                    row.id
                );
                continue;
            }
            if row.binary.is_empty() {
                return Err(DbErr::Custom(format!(
                    "artwork row {} has unsupported body size {}",
                    row.id,
                    row.binary.len()
                )));
            }
            let digest = blake3::hash(&row.binary).to_hex().to_string();
            let id = catalog_identity::artwork(&digest).to_string();
            if !identities.insert(id.clone()) {
                return Err(DbErr::Custom(format!(
                    "ambiguous duplicate artwork identity at row {}",
                    row.id
                )));
            }
            if row.hlc_uuid != id {
                changes.push(("media_cover_art", row.id, id));
            }
        }
    }
    if schema.has_table("media_file_fingerprint").await? {
        let query = Query::select()
            .from_as(Alias::new("media_file_fingerprint"), Alias::new("v"))
            .columns(["id", "media_file_id", "hlc_uuid"].map(|v| (Alias::new("v"), Alias::new(v))))
            .expr_as(
                Expr::col((Alias::new("f"), Alias::new("hlc_uuid"))),
                Alias::new("file_uuid"),
            )
            .join_as(
                JoinType::LeftJoin,
                Alias::new("media_files"),
                Alias::new("f"),
                Expr::col((Alias::new("v"), Alias::new("media_file_id")))
                    .equals((Alias::new("f"), Alias::new("id"))),
            )
            .to_owned();
        let rows = FingerprintIdentityRow::find_by_statement(backend.build(&query))
            .all(tx)
            .await?;
        let mut files = HashSet::new();
        for row in rows {
            if !files.insert(row.media_file_id) {
                return Err(DbErr::Custom(format!(
                    "conflicting legacy fingerprints for file {} at row {}",
                    row.media_file_id, row.id
                )));
            }
            let raw = row
                .file_uuid
                .ok_or_else(|| DbErr::Custom(format!("fingerprint row {} missing file", row.id)))?;
            let file = Uuid::parse_str(&raw).map_err(|e| {
                DbErr::Custom(format!("fingerprint row {} invalid file UUID: {e}", row.id))
            })?;
            if file.is_nil() {
                return Err(DbErr::Custom(format!(
                    "fingerprint row {} has nil file UUID",
                    row.id
                )));
            }
            let id = catalog_identity::fingerprint(file, None, None, None).to_string();
            if row.hlc_uuid != id {
                changes.push(("media_file_fingerprint", row.id, id));
            }
        }
    }
    if schema.has_table("media_file_similarity").await? {
        let query = Query::select()
            .from_as(Alias::new("media_file_similarity"), Alias::new("v"))
            .columns(
                ["id", "file_id1", "file_id2", "hlc_uuid", "similarity"]
                    .map(|v| (Alias::new("v"), Alias::new(v))),
            )
            .expr_as(
                Expr::col((Alias::new("f"), Alias::new("hlc_uuid"))),
                Alias::new("first_uuid"),
            )
            .expr_as(
                Expr::col((Alias::new("s"), Alias::new("hlc_uuid"))),
                Alias::new("second_uuid"),
            )
            .join_as(
                JoinType::LeftJoin,
                Alias::new("media_files"),
                Alias::new("f"),
                Expr::col((Alias::new("v"), Alias::new("file_id1")))
                    .equals((Alias::new("f"), Alias::new("id"))),
            )
            .join_as(
                JoinType::LeftJoin,
                Alias::new("media_files"),
                Alias::new("s"),
                Expr::col((Alias::new("v"), Alias::new("file_id2")))
                    .equals((Alias::new("s"), Alias::new("id"))),
            )
            .to_owned();
        let rows = SimilarityIdentityRow::find_by_statement(backend.build(&query))
            .all(tx)
            .await?;
        let mut pairs = HashSet::new();
        for row in rows {
            if !pairs.insert((row.file_id1, row.file_id2)) {
                return Err(DbErr::Custom(format!(
                    "duplicate similarity pair at row {}",
                    row.id
                )));
            }
            if !row.similarity.is_finite() {
                return Err(DbErr::Custom(format!(
                    "non-finite similarity at row {}",
                    row.id
                )));
            }
            let parse = |value: Option<String>| -> Result<Uuid, DbErr> {
                let raw = value.ok_or_else(|| {
                    DbErr::Custom(format!("similarity row {} missing file", row.id))
                })?;
                let id = Uuid::parse_str(&raw).map_err(|e| {
                    DbErr::Custom(format!("similarity row {} invalid file UUID: {e}", row.id))
                })?;
                if id.is_nil() {
                    return Err(DbErr::Custom(format!(
                        "similarity row {} nil file UUID",
                        row.id
                    )));
                }
                Ok(id)
            };
            let id = catalog_identity::similarity(
                parse(row.first_uuid)?,
                parse(row.second_uuid)?,
                None,
                None,
                None,
            )
            .to_string();
            if row.hlc_uuid != id {
                changes.push(("media_file_similarity", row.id, id));
            }
        }
    }
    if !changes.is_empty() {
        for table in [
            "sync_candidates",
            "sync_journal",
            "sync_checkpoints",
            "sync_objects",
        ] {
            if schema.has_table(table).await? {
                let query = Query::select()
                    .expr(Expr::value(1))
                    .from(Alias::new(table))
                    .limit(1)
                    .to_owned();
                if tx.query_one_raw(backend.build(&query)).await?.is_some() {
                    return Err(DbErr::Custom(format!(
                        "catalog identity correction requires unpublished state; {table} is not empty"
                    )));
                }
            }
        }
        for (table, id, uuid) in changes {
            let query = Query::update()
                .table(Alias::new(table))
                .value(Alias::new("hlc_uuid"), uuid)
                .and_where(Expr::col(Alias::new("id")).eq(id))
                .to_owned();
            tx.execute_raw(backend.build(&query)).await?;
        }
    }
    Ok(())
}

async fn relax_similarity_pair_uniqueness(
    tx: &sea_orm_migration::sea_orm::DatabaseTransaction,
) -> Result<(), DbErr> {
    let schema = SchemaManager::new(tx);
    if !schema.has_table("media_file_similarity").await? {
        return Ok(());
    }
    let old_index = "sqlite_autoindex_media_file_similarity_1";
    if schema.has_index("media_file_similarity", old_index).await? {
        let temp = Alias::new("media_file_similarity_rebuild");
        let mut create = Table::create();
        create
            .table(temp.clone())
            .col(
                ColumnDef::new(Alias::new("id"))
                    .integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .col(ColumnDef::new(Alias::new("file_id1")).integer().not_null())
            .col(ColumnDef::new(Alias::new("file_id2")).integer().not_null())
            .col(ColumnDef::new(Alias::new("similarity")).float().not_null())
            .col(
                ColumnDef::new(Alias::new("hlc_uuid"))
                    .string()
                    .not_null()
                    .default(""),
            );
        let mut legacy_columns = Vec::new();
        for prefix in ["created_at", "updated_at"] {
            for (suffix, kind, default) in [
                ("ts", "text", "1970-01-01T00:00:00Z"),
                ("ver", "integer", "0"),
                ("nid", "text", ""),
            ] {
                let name = format!("{prefix}_hlc_{suffix}");
                if schema.has_column("media_file_similarity", &name).await? {
                    let mut column = ColumnDef::new(Alias::new(&name));
                    if kind == "integer" {
                        column.integer().not_null().default(0);
                    } else {
                        column.text().not_null().default(default);
                    }
                    create.col(&mut column);
                    legacy_columns.push(name);
                }
            }
        }
        for column in ["file_id1", "file_id2"] {
            create.foreign_key(
                ForeignKey::create()
                    .from(temp.clone(), Alias::new(column))
                    .to(Alias::new("media_files"), Alias::new("id"))
                    .on_delete(ForeignKeyAction::Cascade)
                    .on_update(ForeignKeyAction::Cascade),
            );
        }
        schema.create_table(create.to_owned()).await?;
        let mut columns: Vec<String> = ["id", "file_id1", "file_id2", "similarity", "hlc_uuid"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        columns.extend(legacy_columns);
        let select = Query::select()
            .columns(columns.iter().map(Alias::new))
            .from(Alias::new("media_file_similarity"))
            .to_owned();
        let insert = Query::insert()
            .into_table(temp.clone())
            .columns(columns.iter().map(Alias::new))
            .select_from(select)
            .map_err(|e| DbErr::Custom(e.to_string()))?
            .to_owned();
        tx.execute_raw(tx.get_database_backend().build(&insert))
            .await?;
        schema
            .drop_table(
                Table::drop()
                    .table(Alias::new("media_file_similarity"))
                    .to_owned(),
            )
            .await?;
        schema
            .rename_table(
                Table::rename()
                    .table(temp, Alias::new("media_file_similarity"))
                    .to_owned(),
            )
            .await?;
    }
    Ok(())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let txn = db.begin().await?;
        source_uuid_conversion::identities(&txn, "media_files").await?;
        let select = Query::select()
            .from_as(Alias::new("media_file_stats"), Alias::new("s"))
            .columns(
                [
                    "id",
                    "media_file_id",
                    "liked",
                    "skipped",
                    "played_through",
                    "updated_at",
                ]
                .map(|column| (Alias::new("s"), Alias::new(column))),
            )
            .expr_as(
                Expr::col((Alias::new("f"), Alias::new("hlc_uuid"))),
                Alias::new("file_uuid"),
            )
            .join_as(
                JoinType::LeftJoin,
                Alias::new("media_files"),
                Alias::new("f"),
                Expr::col((Alias::new("f"), Alias::new("id")))
                    .equals((Alias::new("s"), Alias::new("media_file_id"))),
            )
            .to_owned();
        let rows = LegacyStat::find_by_statement(db.get_database_backend().build(&select))
            .all(&txn)
            .await?;
        let mut seen = HashSet::new();
        for row in &rows {
            if !seen.insert(row.media_file_id) {
                return Err(DbErr::Custom(format!(
                    "Ambiguous legacy statistics for media file {}",
                    row.media_file_id
                )));
            }
            let file_uuid = row.file_uuid.as_deref().ok_or_else(|| {
                DbErr::Custom("Legacy statistics reference a missing media file".to_owned())
            })?;
            Uuid::parse_str(file_uuid)
                .map_err(|e| DbErr::Custom(format!("Invalid media file UUID: {e}")))?;
        }
        let device_uuid = if rows.is_empty() {
            None
        } else {
            let actor = crate::migration_actor().ok_or_else(|| {
                DbErr::Custom("Migration actor required for legacy statistics".to_owned())
            })?;
            let actor = Uuid::parse_str(&actor)
                .map_err(|e| DbErr::Custom(format!("Invalid migration device UUID: {e}")))?;
            if actor.is_nil() {
                return Err(DbErr::Custom(
                    "Migration device UUID must not be nil".to_owned(),
                ));
            }
            Some(actor)
        };
        let initial_track_sequence = source_uuid_conversion::track_sequence(&txn).await?;
        correct_catalog_identities(&txn).await?;
        relax_similarity_pair_uniqueness(&txn).await?;
        let identity_schema = SchemaManager::new(&txn);
        for (table, columns) in [
            (
                "media_file_fingerprint",
                vec![
                    "source_hash",
                    "source_size",
                    "algorithm",
                    "configuration",
                    "format_version",
                ],
            ),
            (
                "media_file_similarity",
                vec![
                    "first_source_hash",
                    "first_source_size",
                    "second_source_hash",
                    "second_source_size",
                    "algorithm",
                    "configuration",
                    "format_version",
                ],
            ),
        ] {
            if identity_schema.has_table(table).await? {
                for column in columns {
                    let mut def = ColumnDef::new(Alias::new(column));
                    if column.ends_with("size") {
                        def.big_integer();
                    } else if column == "format_version" {
                        def.integer();
                    } else {
                        def.text();
                    }
                    identity_schema
                        .alter_table(
                            Table::alter()
                                .table(Alias::new(table))
                                .add_column(def.to_owned())
                                .to_owned(),
                        )
                        .await?;
                }
            }
        }
        if identity_schema.has_table("media_file_fingerprint").await? {
            identity_schema
                .create_index(
                    Index::create()
                        .name("idx_fingerprint_source_file")
                        .table(Alias::new("media_file_fingerprint"))
                        .col(Alias::new("media_file_id"))
                        .to_owned(),
                )
                .await?;
        }
        for (table, name) in [
            ("media_file_fingerprint", "idx_fingerprint_identity_unique"),
            ("media_file_similarity", "idx_similarity_identity_unique"),
        ] {
            if identity_schema.has_table(table).await? {
                identity_schema
                    .create_index(
                        Index::create()
                            .name(name)
                            .table(Alias::new(table))
                            .col(Alias::new("hlc_uuid"))
                            .unique()
                            .to_owned(),
                    )
                    .await?;
            }
        }
        let schema = SchemaManager::new(&txn);
        for column in [
            ColumnDef::new(Alias::new("local_available"))
                .boolean()
                .not_null()
                .default(true)
                .to_owned(),
            ColumnDef::new(Alias::new("content_hash")).text().to_owned(),
            ColumnDef::new(Alias::new("content_size"))
                .big_integer()
                .to_owned(),
        ] {
            schema
                .alter_table(
                    Table::alter()
                        .table(Alias::new("media_files"))
                        .add_column(column)
                        .to_owned(),
                )
                .await?;
        }
        schema
            .rename_table(
                Table::rename()
                    .table(
                        Alias::new("media_file_stats"),
                        Alias::new("legacy_media_file_stats"),
                    )
                    .to_owned(),
            )
            .await?;
        let mut stats = Table::create();
        stats
            .table(Alias::new("media_file_stats"))
            .col(
                ColumnDef::new(Alias::new("id"))
                    .integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .col(
                ColumnDef::new(Alias::new("media_file_id"))
                    .integer()
                    .not_null(),
            )
            .col(ColumnDef::new(Alias::new("device_id")).text().not_null())
            .col(
                ColumnDef::new(Alias::new("hlc_uuid"))
                    .text()
                    .not_null()
                    .unique_key(),
            )
            .col(ColumnDef::new(Alias::new("skipped")).integer().not_null())
            .col(
                ColumnDef::new(Alias::new("played_through"))
                    .integer()
                    .not_null(),
            )
            .col(ColumnDef::new(Alias::new("updated_at")).text().not_null())
            .index(
                Index::create()
                    .col(Alias::new("media_file_id"))
                    .col(Alias::new("device_id"))
                    .unique(),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(Alias::new("media_file_stats"), Alias::new("media_file_id"))
                    .to(Alias::new("media_files"), Alias::new("id"))
                    .on_delete(ForeignKeyAction::Cascade)
                    .on_update(ForeignKeyAction::Cascade),
            );
        schema.create_table(stats.to_owned()).await?;
        schema
            .create_table(
                Table::create()
                    .table(Alias::new("media_file_likes"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("media_file_id"))
                            .integer()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("hlc_uuid"))
                            .text()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Alias::new("liked")).boolean().not_null())
                    .col(ColumnDef::new(Alias::new("updated_at")).text().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .from(Alias::new("media_file_likes"), Alias::new("media_file_id"))
                            .to(Alias::new("media_files"), Alias::new("id"))
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        for row in rows {
            let file_uuid_text = row.file_uuid.ok_or_else(|| {
                DbErr::Custom("Legacy statistics reference a missing media file".to_owned())
            })?;
            let file_uuid =
                Uuid::parse_str(&file_uuid_text).map_err(|e| DbErr::Custom(e.to_string()))?;
            let device_uuid = device_uuid.ok_or_else(|| {
                DbErr::Custom("Migration actor required for legacy statistics".to_owned())
            })?;
            let device = device_uuid.to_string();
            let stat_uuid = Uuid::new_v5(&file_uuid, device_uuid.as_bytes()).to_string();
            let stats_insert = Query::insert()
                .into_table(Alias::new("media_file_stats"))
                .columns(
                    [
                        "id",
                        "media_file_id",
                        "device_id",
                        "hlc_uuid",
                        "skipped",
                        "played_through",
                        "updated_at",
                    ]
                    .map(Alias::new),
                )
                .values_panic([
                    row.id.into(),
                    row.media_file_id.into(),
                    device.into(),
                    stat_uuid.into(),
                    row.skipped.into(),
                    row.played_through.into(),
                    row.updated_at.clone().into(),
                ])
                .to_owned();
            txn.execute(&stats_insert).await?;
            if row.liked {
                let like_insert = Query::insert()
                    .into_table(Alias::new("media_file_likes"))
                    .columns(["media_file_id", "hlc_uuid", "liked", "updated_at"].map(Alias::new))
                    .values_panic([
                        row.media_file_id.into(),
                        file_uuid_text.into(),
                        row.liked.into(),
                        row.updated_at.into(),
                    ])
                    .to_owned();
                txn.execute(&like_insert).await?;
            }
        }
        schema
            .drop_table(
                Table::drop()
                    .table(Alias::new("legacy_media_file_stats"))
                    .to_owned(),
            )
            .await?;
        source_uuid_conversion::convert(&txn, initial_track_sequence).await?;
        txn.commit().await
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let txn = db.begin().await?;
        if SchemaManager::new(&txn)
            .has_column("media_file_stats", "track_uuid")
            .await?
        {
            return Err(DbErr::Custom("UUID relationship conversion cannot be rolled back without resolving every reference and device owner".to_owned()));
        }
        #[derive(FromQueryResult)]
        struct FileId {
            media_file_id: i32,
        }
        let stats_select = Query::select()
            .column(Alias::new("media_file_id"))
            .from(Alias::new("media_file_stats"))
            .to_owned();
        let likes_select = Query::select()
            .column(Alias::new("media_file_id"))
            .from(Alias::new("media_file_likes"))
            .to_owned();
        let stats = FileId::find_by_statement(db.get_database_backend().build(&stats_select))
            .all(&txn)
            .await?;
        let likes = FileId::find_by_statement(db.get_database_backend().build(&likes_select))
            .all(&txn)
            .await?;
        let mut ids = HashSet::new();
        if stats.iter().any(|row| !ids.insert(row.media_file_id))
            || likes.iter().any(|row| !ids.contains(&row.media_file_id))
        {
            return Err(DbErr::Custom(
                "Cannot roll back multiple device statistics or likes without statistics"
                    .to_owned(),
            ));
        }
        let schema = SchemaManager::new(&txn);
        schema
            .create_table(
                Table::create()
                    .table(Alias::new("legacy_media_file_stats"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("media_file_id"))
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Alias::new("liked")).boolean().not_null())
                    .col(ColumnDef::new(Alias::new("skipped")).integer().not_null())
                    .col(
                        ColumnDef::new(Alias::new("played_through"))
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Alias::new("updated_at")).text().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .from(
                                Alias::new("legacy_media_file_stats"),
                                Alias::new("media_file_id"),
                            )
                            .to(Alias::new("media_files"), Alias::new("id"))
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        let legacy_rows = Query::select()
            .from_as(Alias::new("media_file_stats"), Alias::new("s"))
            .columns(["id", "media_file_id"].map(|column| (Alias::new("s"), Alias::new(column))))
            .expr(Func::coalesce([
                Expr::col((Alias::new("f"), Alias::new("liked"))),
                Expr::val(false),
            ]))
            .columns(
                ["skipped", "played_through", "updated_at"]
                    .map(|column| (Alias::new("s"), Alias::new(column))),
            )
            .join_as(
                JoinType::LeftJoin,
                Alias::new("media_file_likes"),
                Alias::new("f"),
                Expr::col((Alias::new("f"), Alias::new("media_file_id")))
                    .equals((Alias::new("s"), Alias::new("media_file_id"))),
            )
            .to_owned();
        let insert = Query::insert()
            .into_table(Alias::new("legacy_media_file_stats"))
            .columns(
                [
                    "id",
                    "media_file_id",
                    "liked",
                    "skipped",
                    "played_through",
                    "updated_at",
                ]
                .map(Alias::new),
            )
            .select_from(legacy_rows)
            .map_err(|error| DbErr::Custom(error.to_string()))?
            .to_owned();
        txn.execute(&insert).await?;
        schema
            .drop_table(
                Table::drop()
                    .table(Alias::new("media_file_stats"))
                    .to_owned(),
            )
            .await?;
        schema
            .rename_table(
                Table::rename()
                    .table(
                        Alias::new("legacy_media_file_stats"),
                        Alias::new("media_file_stats"),
                    )
                    .to_owned(),
            )
            .await?;
        schema
            .drop_table(
                Table::drop()
                    .table(Alias::new("media_file_likes"))
                    .to_owned(),
            )
            .await?;
        for column in ["content_size", "content_hash", "local_available"] {
            schema
                .alter_table(
                    Table::alter()
                        .table(Alias::new("media_files"))
                        .drop_column(Alias::new(column))
                        .to_owned(),
                )
                .await?;
        }
        for (table, name) in [
            ("media_file_fingerprint", "idx_fingerprint_identity_unique"),
            ("media_file_similarity", "idx_similarity_identity_unique"),
        ] {
            if schema.has_table(table).await? {
                schema
                    .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                    .await?;
            }
        }
        if schema.has_table("media_file_similarity").await? {
            schema
                .create_index(
                    Index::create()
                        .name("idx_file_ids_legacy")
                        .table(Alias::new("media_file_similarity"))
                        .col(Alias::new("file_id1"))
                        .col(Alias::new("file_id2"))
                        .unique()
                        .to_owned(),
                )
                .await?;
        }
        if schema.has_table("media_file_similarity").await?
            && !schema
                .has_index(
                    "media_file_similarity",
                    "idx_media_file_similarity_hlc_uuid",
                )
                .await?
        {
            schema
                .create_index(
                    Index::create()
                        .name("idx_media_file_similarity_hlc_uuid")
                        .table(Alias::new("media_file_similarity"))
                        .col(Alias::new("hlc_uuid"))
                        .to_owned(),
                )
                .await?;
        }
        if schema.has_table("media_file_fingerprint").await? {
            schema
                .drop_index(
                    Index::drop()
                        .name("idx_fingerprint_source_file")
                        .table(Alias::new("media_file_fingerprint"))
                        .to_owned(),
                )
                .await?;
        }
        for (table, columns) in [
            (
                "media_file_fingerprint",
                vec![
                    "source_hash",
                    "source_size",
                    "algorithm",
                    "configuration",
                    "format_version",
                ],
            ),
            (
                "media_file_similarity",
                vec![
                    "first_source_hash",
                    "first_source_size",
                    "second_source_hash",
                    "second_source_size",
                    "algorithm",
                    "configuration",
                    "format_version",
                ],
            ),
        ] {
            if schema.has_table(table).await? {
                for column in columns {
                    schema
                        .alter_table(
                            Table::alter()
                                .table(Alias::new(table))
                                .drop_column(Alias::new(column))
                                .to_owned(),
                        )
                        .await?;
                }
            }
        }
        txn.commit().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_identity::ParentKind;
    use sea_orm_migration::sea_orm::{ConnectionTrait, Database, Statement};

    async fn setup() -> sea_orm_migration::sea_orm::DatabaseConnection {
        let _ = crate::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned());
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE media_files (id INTEGER PRIMARY KEY, hlc_uuid TEXT NOT NULL)",
        )
        .await
        .unwrap();
        db.execute_unprepared("CREATE TABLE media_file_stats (id INTEGER PRIMARY KEY, media_file_id INTEGER NOT NULL, liked BOOLEAN NOT NULL, skipped INTEGER NOT NULL, played_through INTEGER NOT NULL, updated_at TEXT NOT NULL)").await.unwrap();
        db
    }
    #[tokio::test]
    async fn sync_migrations_run_in_transactions() {
        let _ = crate::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned());
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let position = crate::Migrator::migrations()
            .iter()
            .position(|m| m.name() == "m20261101_000034_create_download_rules")
            .unwrap();
        crate::Migrator::up(&db, Some(position as u32))
            .await
            .unwrap();
        db.execute_unprepared("CREATE TABLE sync_download_jobs (id TEXT)")
            .await
            .unwrap();
        assert!(crate::Migrator::up(&db, None).await.is_err());
        assert!(
            !SchemaManager::new(&db)
                .has_table("sync_download_rules")
                .await
                .unwrap()
        );
        let record = db.query_one_raw(Statement::from_string(db.get_database_backend(), "SELECT version FROM seaql_migrations WHERE version='m20261101_000034_create_download_rules'".to_owned())).await.unwrap();
        assert!(record.is_none());
    }
    #[tokio::test]
    async fn legacy_unliked_rows_do_not_create_favorites() {
        let db = setup().await;
        db.execute_unprepared(
            "INSERT INTO media_files VALUES (7, 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')",
        )
        .await
        .unwrap();
        db.execute_unprepared("INSERT INTO media_file_stats VALUES (4, 7, 0, 3, 9, '2020-01-01')")
            .await
            .unwrap();
        crate::with_migration_actor(
            Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            Migration.up(&SchemaManager::new(&db)),
        )
        .await
        .unwrap();
        let row = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT COUNT(*) AS n FROM media_file_likes".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 0);
    }
    #[tokio::test]
    async fn fresh_database_migrates() {
        let _ = crate::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned());
        let db = Database::connect("sqlite::memory:").await.unwrap();
        crate::Migrator::up(&db, None).await.unwrap();
        let manager = SchemaManager::new(&db);
        assert!(manager.has_table("media_file_likes").await.unwrap());
        assert!(manager.has_table("sync_download_jobs").await.unwrap());
        assert!(!manager.has_table("sync_remote_catalog").await.unwrap());
    }
    #[tokio::test]
    async fn preserves_legacy_data() {
        let db = setup().await;
        db.execute_unprepared(
            "INSERT INTO media_files VALUES (7, 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')",
        )
        .await
        .unwrap();
        db.execute_unprepared("INSERT INTO media_file_stats VALUES (4, 7, 1, 3, 9, '2020-01-01')")
            .await
            .unwrap();
        crate::with_migration_actor(
            Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            Migration.up(&SchemaManager::new(&db)),
        )
        .await
        .unwrap();
        let row = db.query_one_raw(Statement::from_string(db.get_database_backend(), "SELECT s.skipped, s.played_through, s.device_id, s.hlc_uuid, f.liked, m.local_available FROM media_file_stats s JOIN media_file_likes f ON f.track_uuid=s.track_uuid JOIN media_files m ON m.hlc_uuid=s.track_uuid".to_owned())).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i32>("", "skipped").unwrap(), 3);
        assert_eq!(row.try_get::<i32>("", "played_through").unwrap(), 9);
        assert_eq!(
            row.try_get::<String>("", "device_id").unwrap(),
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
        );
        let expected = Uuid::new_v5(
            &Uuid::parse_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap(),
            Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
                .unwrap()
                .as_bytes(),
        )
        .to_string();
        assert_eq!(row.try_get::<String>("", "hlc_uuid").unwrap(), expected);
        assert!(row.try_get::<bool>("", "liked").unwrap());
        assert!(row.try_get::<bool>("", "local_available").unwrap());
        assert!(Migration.down(&SchemaManager::new(&db)).await.is_err());
        let restored = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT f.liked, s.skipped, s.played_through FROM media_file_stats s JOIN media_file_likes f ON f.track_uuid=s.track_uuid WHERE s.id=4".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert!(restored.try_get::<bool>("", "liked").unwrap());
        assert_eq!(restored.try_get::<i32>("", "skipped").unwrap(), 3);
        assert_eq!(restored.try_get::<i32>("", "played_through").unwrap(), 9);
    }
    #[tokio::test]
    async fn orphan_legacy_rows_roll_back() {
        let db = setup().await;
        db.execute_unprepared("INSERT INTO media_file_stats VALUES (4, 99, 1, 3, 9, '2020-01-01')")
            .await
            .unwrap();
        assert!(Migration.up(&SchemaManager::new(&db)).await.is_err());
        let row = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT COUNT(*) AS n FROM media_file_stats".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 1);
        assert!(
            !SchemaManager::new(&db)
                .has_table("media_file_likes")
                .await
                .unwrap()
        );
    }
    #[tokio::test]
    async fn duplicate_legacy_rows_roll_back() {
        let db = setup().await;
        db.execute_unprepared(
            "INSERT INTO media_files VALUES (7, 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')",
        )
        .await
        .unwrap();
        db.execute_unprepared("INSERT INTO media_file_stats VALUES (4, 7, 1, 3, 9, '2020-01-01'), (5, 7, 0, 0, 0, '2020-01-02')").await.unwrap();
        assert!(Migration.up(&SchemaManager::new(&db)).await.is_err());
        let row = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT COUNT(*) AS n FROM media_file_stats".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 2);
        assert!(
            !SchemaManager::new(&db)
                .has_table("media_file_likes")
                .await
                .unwrap()
        );
    }
    async fn populated_library() -> sea_orm_migration::sea_orm::DatabaseConnection {
        let _ = crate::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned());
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let position = crate::Migrator::migrations()
            .iter()
            .position(|m| m.name() == "m20261101_000032_create_sync_library")
            .unwrap();
        crate::Migrator::up(&db, Some(position as u32))
            .await
            .unwrap();
        db.execute_unprepared("INSERT INTO media_files(id,file_name,directory,extension,file_hash,last_modified,sample_rate,duration,hlc_uuid) VALUES(7,'a','/music','ogg','crc','2020',44100,12,'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')").await.unwrap();
        db.execute_unprepared("INSERT INTO media_file_stats(id,media_file_id,liked,skipped,played_through,updated_at) VALUES(4,7,1,3,9,'2020-01-01')").await.unwrap();
        db
    }

    async fn assert_owner(db: &sea_orm_migration::sea_orm::DatabaseConnection, actor: &str) {
        let row = db.query_one_raw(Statement::from_string(db.get_database_backend(), "SELECT s.device_id,s.hlc_uuid,s.skipped,s.played_through,f.liked FROM media_file_stats s JOIN media_file_likes f ON s.track_uuid=f.track_uuid".to_owned())).await.unwrap().unwrap();
        assert_eq!(row.try_get::<String>("", "device_id").unwrap(), actor);
        let file = Uuid::parse_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap();
        assert_eq!(
            row.try_get::<String>("", "hlc_uuid").unwrap(),
            Uuid::new_v5(&file, Uuid::parse_str(actor).unwrap().as_bytes()).to_string()
        );
        assert_eq!(row.try_get::<i32>("", "skipped").unwrap(), 3);
        assert_eq!(row.try_get::<i32>("", "played_through").unwrap(), 9);
        assert!(row.try_get::<bool>("", "liked").unwrap());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn distinct_library_actors_are_scoped_sequentially_and_concurrently() {
        let actor_a = "11111111-1111-4111-8111-111111111111";
        let actor_b = "22222222-2222-4222-8222-222222222222";
        let a = populated_library().await;
        let b = populated_library().await;
        crate::up_with_actor(&a, Some(actor_a), None).await.unwrap();
        crate::up_with_actor(&b, Some(actor_b), None).await.unwrap();
        assert_owner(&a, actor_a).await;
        assert_owner(&b, actor_b).await;
        let c = populated_library().await;
        let d = populated_library().await;
        let c_task = c.clone();
        let d_task = d.clone();
        let (c_result, d_result) = tokio::join!(
            tokio::spawn(async move { crate::up_with_actor(&c_task, Some(actor_a), None).await }),
            tokio::spawn(async move { crate::up_with_actor(&d_task, Some(actor_b), None).await })
        );
        c_result.unwrap().unwrap();
        d_result.unwrap().unwrap();
        assert_owner(&c, actor_a).await;
        assert_owner(&d, actor_b).await;
    }

    #[tokio::test]
    async fn missing_or_invalid_actor_preserves_populated_library() {
        for actor in [
            None,
            Some("invalid"),
            Some("00000000-0000-0000-0000-000000000000"),
        ] {
            let db = populated_library().await;
            assert!(crate::up_with_actor(&db, actor, None).await.is_err());
            let schema = SchemaManager::new(&db);
            assert!(!schema.has_table("media_file_likes").await.unwrap());
            assert!(
                !schema
                    .has_column("media_files", "local_available")
                    .await
                    .unwrap()
            );
            let row = db
                .query_one_raw(Statement::from_string(
                    db.get_database_backend(),
                    "SELECT liked, skipped, played_through FROM media_file_stats WHERE id=4"
                        .to_owned(),
                ))
                .await
                .unwrap()
                .unwrap();
            assert!(row.try_get::<bool>("", "liked").unwrap());
            assert_eq!(row.try_get::<i32>("", "skipped").unwrap(), 3);
            assert_eq!(row.try_get::<i32>("", "played_through").unwrap(), 9);
        }
    }

    async fn setup_catalog() -> sea_orm_migration::sea_orm::DatabaseConnection {
        let db = setup().await;
        db.execute_unprepared("CREATE TABLE artists (id INTEGER PRIMARY KEY, name TEXT NOT NULL, hlc_uuid TEXT NOT NULL)").await.unwrap();
        db.execute_unprepared("CREATE TABLE media_file_artists (id INTEGER PRIMARY KEY, media_file_id INTEGER NOT NULL, artist_id INTEGER NOT NULL, hlc_uuid TEXT NOT NULL)").await.unwrap();
        db.execute_unprepared(
            "INSERT INTO media_files VALUES (7, 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')",
        )
        .await
        .unwrap();
        db.execute_unprepared("INSERT INTO artists VALUES (3, 'Artist', 'old-artist')")
            .await
            .unwrap();
        db.execute_unprepared("INSERT INTO media_file_artists VALUES (5, 7, 3, 'old-link')")
            .await
            .unwrap();
        db
    }
    #[tokio::test]
    async fn corrects_catalog_identities_preserving_keys() {
        let db = setup_catalog().await;
        Migration.up(&SchemaManager::new(&db)).await.unwrap();
        let row = db.query_one_raw(Statement::from_string(db.get_database_backend(), "SELECT a.id AS artist_id, a.hlc_uuid AS artist_uuid, l.id AS link_id, l.track_uuid, l.artist_uuid AS link_artist_uuid, l.hlc_uuid AS link_uuid FROM artists a JOIN media_file_artists l ON l.artist_uuid=a.hlc_uuid".to_owned())).await.unwrap().unwrap();
        let parent = crate::catalog_identity::parent(ParentKind::Artist, "Artist");
        let file = Uuid::parse_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap();
        assert_eq!(row.try_get::<i32>("", "artist_id").unwrap(), 3);
        assert_eq!(row.try_get::<i32>("", "link_id").unwrap(), 5);
        assert_eq!(
            row.try_get::<String>("", "track_uuid").unwrap(),
            file.to_string()
        );
        assert_eq!(
            row.try_get::<String>("", "link_artist_uuid").unwrap(),
            parent.to_string()
        );
        assert_eq!(
            row.try_get::<String>("", "artist_uuid").unwrap(),
            parent.to_string()
        );
        assert_eq!(
            row.try_get::<String>("", "link_uuid").unwrap(),
            crate::catalog_identity::membership(file, ParentKind::Artist, parent).to_string()
        );
    }
    #[tokio::test]
    async fn catalog_correction_refuses_published_state() {
        let db = setup_catalog().await;
        db.execute_unprepared("CREATE TABLE sync_objects (id INTEGER)")
            .await
            .unwrap();
        db.execute_unprepared("INSERT INTO sync_objects VALUES (1)")
            .await
            .unwrap();
        assert!(Migration.up(&SchemaManager::new(&db)).await.is_err());
        let row = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT hlc_uuid FROM artists WHERE id=3".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "hlc_uuid").unwrap(), "old-artist");
    }
    #[tokio::test]
    async fn catalog_correction_rejects_duplicate_pairs() {
        let db = setup_catalog().await;
        db.execute_unprepared("INSERT INTO media_file_artists VALUES (6, 7, 3, 'second-link')")
            .await
            .unwrap();
        assert!(Migration.up(&SchemaManager::new(&db)).await.is_err());
        let row = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT COUNT(*) AS n FROM media_file_artists".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 2);
    }
    #[tokio::test]
    async fn artwork_uses_body_digest_and_preserves_crc_collisions() {
        let db = setup().await;
        db.execute_unprepared("CREATE TABLE media_cover_art (id INTEGER PRIMARY KEY, file_hash TEXT NOT NULL, binary BLOB NOT NULL, hlc_uuid TEXT NOT NULL)").await.unwrap();
        db.execute_unprepared("INSERT INTO media_cover_art(id,file_hash,binary,hlc_uuid) VALUES (1,'same',x'010203','old-a'), (2,'same',x'040506','old-b'), (3,'',x'','00000000-0000-0000-0000-000000000000')").await.unwrap();
        Migration.up(&SchemaManager::new(&db)).await.unwrap();
        let rows = db
            .query_all_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT id,hlc_uuid FROM media_cover_art ORDER BY id".to_owned(),
            ))
            .await
            .unwrap();
        for (i, bytes) in [&[1u8, 2, 3][..], &[4u8, 5, 6][..]].into_iter().enumerate() {
            let expected = crate::catalog_identity::artwork(blake3::hash(bytes).to_hex().as_ref());
            assert_eq!(
                rows[i].try_get::<String>("", "hlc_uuid").unwrap(),
                expected.to_string()
            );
        }
        assert_eq!(
            rows[2].try_get::<String>("", "hlc_uuid").unwrap(),
            Uuid::nil().to_string()
        );
    }
    #[tokio::test]
    async fn artwork_duplicate_identity_rejects_without_deletion() {
        let db = setup().await;
        db.execute_unprepared("CREATE TABLE media_cover_art (id INTEGER PRIMARY KEY, file_hash TEXT NOT NULL, binary BLOB NOT NULL, hlc_uuid TEXT NOT NULL)").await.unwrap();
        db.execute_unprepared("INSERT INTO media_cover_art VALUES (1,'a',x'010203','old-a'), (2,'b',x'010203','old-b')").await.unwrap();
        assert!(Migration.up(&SchemaManager::new(&db)).await.is_err());
        let row = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT COUNT(*) AS n FROM media_cover_art".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 2);
    }
    async fn setup_analysis_sources() -> sea_orm_migration::sea_orm::DatabaseConnection {
        let db = setup().await;
        db.execute_unprepared("INSERT INTO media_files VALUES (7, 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'), (8, 'cccccccc-cccc-4ccc-8ccc-cccccccccccc')").await.unwrap();
        db.execute_unprepared("CREATE TABLE media_file_fingerprint (id INTEGER PRIMARY KEY, media_file_id INTEGER NOT NULL, fingerprint BLOB NOT NULL, is_duplicated INTEGER NOT NULL, hlc_uuid TEXT NOT NULL)").await.unwrap();
        db.execute_unprepared("CREATE TABLE media_file_similarity (id INTEGER PRIMARY KEY, file_id1 INTEGER NOT NULL, file_id2 INTEGER NOT NULL, similarity REAL NOT NULL, hlc_uuid TEXT NOT NULL)").await.unwrap();
        db
    }
    #[tokio::test]
    async fn analysis_identities_preserve_unknown_legacy_data_and_order() {
        let db = setup_analysis_sources().await;
        db.execute_unprepared("INSERT INTO media_file_fingerprint VALUES(5,7,x'010203',1,'old')")
            .await
            .unwrap();
        db.execute_unprepared(
            "INSERT INTO media_file_similarity VALUES(6,7,8,0.25,'old'), (9,8,7,0.75,'other')",
        )
        .await
        .unwrap();
        Migration.up(&SchemaManager::new(&db)).await.unwrap();
        let rows = db
            .query_all_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT id,hlc_uuid,similarity,algorithm FROM media_file_similarity ORDER BY id"
                    .to_owned(),
            ))
            .await
            .unwrap();
        let first = Uuid::parse_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap();
        let second = Uuid::parse_str("cccccccc-cccc-4ccc-8ccc-cccccccccccc").unwrap();
        assert_eq!(
            rows[0].try_get::<String>("", "hlc_uuid").unwrap(),
            crate::catalog_identity::similarity(first, second, None, None, None).to_string()
        );
        assert_eq!(
            rows[1].try_get::<String>("", "hlc_uuid").unwrap(),
            crate::catalog_identity::similarity(second, first, None, None, None).to_string()
        );
        assert_eq!(
            rows[0].try_get::<Option<String>>("", "algorithm").unwrap(),
            None
        );
        let row = db.query_one_raw(Statement::from_string(db.get_database_backend(), "SELECT id,hlc_uuid,fingerprint,is_duplicated,source_hash FROM media_file_fingerprint".to_owned())).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i32>("", "id").unwrap(), 5);
        assert_eq!(
            row.try_get::<Vec<u8>>("", "fingerprint").unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(row.try_get::<i32>("", "is_duplicated").unwrap(), 1);
        assert_eq!(
            row.try_get::<Option<String>>("", "source_hash").unwrap(),
            None
        );
        assert_eq!(
            row.try_get::<String>("", "hlc_uuid").unwrap(),
            crate::catalog_identity::fingerprint(first, None, None, None).to_string()
        );
    }
    #[tokio::test]
    async fn analysis_duplicate_fingerprints_abort_without_data_loss() {
        let db = setup_analysis_sources().await;
        db.execute_unprepared(
            "INSERT INTO media_file_fingerprint VALUES(5,7,x'01',1,'a'),(6,7,x'02',0,'b')",
        )
        .await
        .unwrap();
        assert!(Migration.up(&SchemaManager::new(&db)).await.is_err());
        let row = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT COUNT(*) AS n FROM media_file_fingerprint".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 2);
    }
    #[tokio::test]
    async fn final_similarity_schema_allows_distinct_identities_and_preserves_children() {
        let _ = crate::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned());
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("PRAGMA foreign_keys=ON")
            .await
            .unwrap();
        crate::Migrator::up(&db, None).await.unwrap();
        db.execute_unprepared("INSERT INTO media_files(id,file_name,directory,extension,file_hash,last_modified,sample_rate,duration,hlc_uuid) VALUES(1,'a','','ogg','a','old',44100,1,'11111111-1111-4111-8111-111111111111'),(2,'b','','ogg','b','old',44100,1,'22222222-2222-4222-8222-222222222222')").await.unwrap();
        db.execute_unprepared("INSERT INTO media_file_similarity(first_track_uuid,second_track_uuid,similarity,hlc_uuid) VALUES('11111111-1111-4111-8111-111111111111','22222222-2222-4222-8222-222222222222',0.1,'first'), ('11111111-1111-4111-8111-111111111111','22222222-2222-4222-8222-222222222222',0.2,'second')").await.unwrap();
        assert!(db.execute_unprepared("INSERT INTO media_file_similarity(first_track_uuid,second_track_uuid,similarity,hlc_uuid) VALUES('11111111-1111-4111-8111-111111111111','22222222-2222-4222-8222-222222222222',0.3,'first')").await.is_err());
        db.execute_unprepared("DELETE FROM media_files WHERE id=1")
            .await
            .unwrap();
        let row = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                "SELECT COUNT(*) AS n FROM media_file_similarity".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 2);
    }
}
