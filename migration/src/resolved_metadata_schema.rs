//! Table definitions for the resolved music catalog.

use sea_orm_migration::{
    prelude::*,
    sea_orm::ConnectionTrait,
    sea_query::{BinOper, IndexCreateStatement, SelectStatement, TableCreateStatement},
};

pub(super) async fn install(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for table in tables() {
        manager.create_table(table).await?;
    }
    for index in indexes() {
        manager.create_index(index).await?;
    }
    // The SQLite backend rejects IndexColumn::Expr, including per-column COLLATE.
    // These indexes keep their expression keys instead of changing their semantics.
    for index in [
        "CREATE INDEX music_metadata_album_field_lookup ON music_metadata_album_field_values(field, value COLLATE NOCASE, album_id) WHERE decision = 1;",
        "CREATE INDEX music_metadata_album_summary_title ON music_metadata_album_summaries(title COLLATE NOCASE, album_id);",
        "CREATE INDEX music_metadata_audio_disc_tracks ON music_metadata_audio(disc_id, (track_number IS NULL), track_number, file_id);",
        "CREATE INDEX music_metadata_local_overlay_lookup ON music_metadata_local_overlays(field, value COLLATE NOCASE, album_id);",
        "CREATE INDEX music_metadata_artist_name ON music_metadata_artists(name COLLATE NOCASE, source_uuid);",
        "CREATE INDEX music_metadata_credits_lookup ON music_metadata_credits(scope, role, credited_as COLLATE NOCASE, file_id);",
        "CREATE INDEX music_metadata_values_lookup ON music_metadata_values(scope, field, qualifier, value COLLATE NOCASE, file_id);",
    ] {
        manager.get_connection().execute_unprepared(index).await?;
    }
    Ok(())
}

pub(super) async fn uninstall(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // This index belongs to a table the catalog migration does not own.
    manager
        .drop_index(
            Index::drop()
                .name("music_metadata_file_digest")
                .table(alias("media_files"))
                .to_owned(),
        )
        .await?;

    // Reverse creation order so each dependent table goes before its parent.
    for table in tables().into_iter().rev() {
        let name = table
            .get_table_name()
            .ok_or_else(|| DbErr::Custom("catalog table has no name".into()))?;
        manager
            .drop_table(Table::drop().table(name.clone()).to_owned())
            .await?;
    }
    Ok(())
}

pub(super) async fn move_table(
    manager: &SchemaManager<'_>,
    from: &str,
    to: &str,
) -> Result<(), DbErr> {
    // Renaming preserves row IDs, tag order, source UUIDs and existing indexes.
    manager
        .rename_table(Table::rename().table(alias(from), alias(to)).to_owned())
        .await
}

pub(super) async fn migrate_artists(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let columns = artist_columns(manager, "artists", "music_metadata_artists").await?;
    copy_artists(
        manager,
        "artists",
        "music_metadata_artists",
        &columns,
        false,
    )
    .await?;
    move_table(
        manager,
        "media_file_artists",
        "music_metadata_audio_artists",
    )
    .await?;

    // Existing browse keys are reused without matching artist spellings.
    insert_selection(
        manager,
        "music_metadata_browse_handles",
        &["kind", "source_uuid"],
        Query::select()
            .exprs([Expr::val("artist"), col("source_uuid")])
            .from(alias("music_metadata_artists"))
            .order_by(alias("id"), Order::Asc)
            .to_owned(),
    )
    .await?;
    manager
        .drop_table(Table::drop().table(alias("artists")).to_owned())
        .await
}

const ALBUM_FIELDS: [(&str, &str); 4] = [
    ("id", "legacy_id"),
    ("source_uuid", "id"),
    ("name", "portable_title"),
    ("group", "portable_group"),
];

pub(super) async fn migrate_albums(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    copy_albums(manager, false).await?;
    manager
        .drop_table(Table::drop().table(alias("albums")).to_owned())
        .await
}

pub(super) async fn restore_albums(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(alias("albums"))
                .col(integer("id").primary_key().auto_increment())
                .col(text("name").not_null().unique_key())
                .col(text("group").not_null())
                .col(text("source_uuid").not_null().default(""))
                .to_owned(),
        )
        .await?;
    copy_albums(manager, true).await?;
    for (name, unique) in [("p2_album_uuid", true), ("idx_albums_source_uuid", false)] {
        let mut index = Index::create();
        index
            .name(name)
            .table(alias("albums"))
            .col(alias("source_uuid"));
        if unique {
            index.unique();
        }
        manager.create_index(index).await?;
    }
    Ok(())
}

async fn copy_albums(manager: &SchemaManager<'_>, restore: bool) -> Result<(), DbErr> {
    let (source, target) = if restore {
        ("music_metadata_albums", "albums")
    } else {
        ("albums", "music_metadata_albums")
    };
    let mut fields = ALBUM_FIELDS.to_vec();
    append_retired_columns(manager, source, target, |name| fields.push((name, name))).await?;

    // Keep the saved handles and sync fields on the same row as the edition.
    let mut rows = Query::select();
    let mut columns = Vec::new();
    for (old, current) in fields {
        let (from, to) = if restore {
            (current, old)
        } else {
            (old, current)
        };
        rows.column(alias(from));
        columns.push(to);
    }
    rows.from(alias(source));
    if restore {
        rows.and_where(col("portable_title").is_not_null());
    }
    insert_selection(manager, target, &columns, rows).await
}

pub(super) async fn restore_artists(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // A rollback restores the portable artist rows and their original numeric IDs.
    manager
        .create_table(
            Table::create()
                .table(alias("artists"))
                .col(integer("id").primary_key().auto_increment())
                .col(text("name").not_null().unique_key())
                .col(text("group").not_null())
                .col(text("source_uuid").not_null().default(""))
                .to_owned(),
        )
        .await?;
    let columns = artist_columns(manager, "music_metadata_artists", "artists").await?;
    copy_artists(manager, "music_metadata_artists", "artists", &columns, true).await?;
    for (name, unique) in [("p2_artist_uuid", true), ("idx_artists_source_uuid", false)] {
        let mut index = Index::create();
        index
            .name(name)
            .table(alias("artists"))
            .col(alias("source_uuid"));
        if unique {
            index.unique();
        }
        manager.create_index(index).await?;
    }
    move_table(
        manager,
        "music_metadata_audio_artists",
        "media_file_artists",
    )
    .await
}

async fn artist_columns(
    manager: &SchemaManager<'_>,
    source: &str,
    target: &str,
) -> Result<Vec<&'static str>, DbErr> {
    let mut columns = vec!["id", "name", "group", "source_uuid"];
    append_retired_columns(manager, source, target, |name| columns.push(name)).await?;
    Ok(columns)
}

async fn append_retired_columns(
    manager: &SchemaManager<'_>,
    source: &str,
    target: &str,
    mut append: impl FnMut(&'static str),
) -> Result<(), DbErr> {
    // Pre-release libraries can carry retired bookkeeping columns with real values.
    for name in [
        "created_at_hlc_ts",
        "created_at_hlc_ver",
        "created_at_hlc_nid",
        "updated_at_hlc_ts",
        "updated_at_hlc_ver",
        "updated_at_hlc_nid",
    ] {
        if manager.has_column(source, name).await? {
            let mut field = if name.ends_with("_ver") {
                integer(name)
            } else {
                text(name)
            };
            manager
                .alter_table(
                    Table::alter()
                        .table(alias(target))
                        .add_column(&mut field)
                        .to_owned(),
                )
                .await?;
            append(name);
        }
    }
    Ok(())
}

async fn copy_artists(
    manager: &SchemaManager<'_>,
    source: &str,
    target: &str,
    columns: &[&str],
    portable_only: bool,
) -> Result<(), DbErr> {
    let mut rows = Query::select();
    rows.columns(columns.iter().map(|name| alias(name)))
        .from(alias(source));
    if portable_only {
        rows.and_where(col("portable").eq(1));
    }
    insert_selection(manager, target, columns, rows).await
}

async fn insert_selection(
    manager: &SchemaManager<'_>,
    target: &str,
    columns: &[&str],
    rows: SelectStatement,
) -> Result<(), DbErr> {
    let insert = Query::insert()
        .into_table(alias(target))
        .columns(columns.iter().map(|name| alias(name)))
        .select_from(rows)
        .map_err(|error| DbErr::Custom(error.to_string()))?
        .to_owned();
    let db = manager.get_connection();
    db.execute_raw(db.get_database_backend().build(&insert))
        .await?;
    Ok(())
}

fn alias(name: &str) -> Alias {
    Alias::new(name)
}

fn col(name: &str) -> Expr {
    Expr::col(alias(name))
}

fn text(name: &str) -> ColumnDef {
    ColumnDef::new(alias(name)).text().to_owned()
}

fn integer(name: &str) -> ColumnDef {
    ColumnDef::new(alias(name)).integer().to_owned()
}

fn real(name: &str) -> ColumnDef {
    // The SQLite builder uses the DOUBLE spelling for its native float type.
    ColumnDef::new(alias(name)).custom(alias("REAL")).to_owned()
}

// SeaQuery has no SQLite WITHOUT ROWID option. Its table suffix preserves the existing layout.
fn tables() -> [TableCreateStatement; 29] {
    [
        Table::create()
            .table(alias("music_metadata_album_groups"))
            .col(text("id").primary_key().not_null())
            .col(text("name").not_null())
            .extra("WITHOUT ROWID")
            .to_owned(),
        Table::create()
            .table(alias("music_metadata_albums"))
            .col(integer("legacy_id").primary_key().auto_increment())
            .col(text("id").not_null())
            .col(text("portable_title").unique_key())
            .col(text("portable_group"))
            .col(text("directory_path").not_null().default(""))
            .col(text("managed_key"))
            .col(
                text("directory_scope").generated(
                    Expr::case(
                        col("portable_title").is_not_null(),
                        Expr::val("portable:").binary(BinOper::Custom("||"), col("id")),
                    )
                    .finally(Func::coalesce([
                        Expr::val("managed:").binary(BinOper::Custom("||"), col("managed_key")),
                        col("directory_path"),
                    ])),
                    false,
                ),
            )
            .col(text("group_id"))
            .col(
                integer("needs_review")
                    .check(col("needs_review").is_in([0, 1]))
                    .default(0)
                    .not_null(),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_albums"), alias("group_id"))
                    .to(alias("music_metadata_album_groups"), alias("id"))
                    .on_delete(ForeignKeyAction::SetNull),
            )
            .to_owned(),
        // Provider proposals are owned by the edition and survive rescanning its files.
        Table::create()
            .table(alias("music_metadata_album_field_values"))
            .col(text("album_id").not_null())
            .col(
                text("field")
                    .check(col("field").is_in([
                        "title",
                        "version",
                        "catalog_number",
                        "release_date",
                        "original_release_date",
                        "labels",
                        "release_type",
                        "barcode",
                        "release_country",
                        "description",
                    ]))
                    .not_null(),
            )
            .col(text("provider").not_null())
            .col(text("resource").not_null())
            .col(text("value"))
            .col(
                integer("decision")
                    .check(col("decision").is_in([0, 1, 2]))
                    .not_null(),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(
                        alias("music_metadata_album_field_values"),
                        alias("album_id"),
                    )
                    .to(alias("music_metadata_albums"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .primary_key(
                Index::create()
                    .col(alias("album_id"))
                    .col(alias("field"))
                    .col(alias("provider"))
                    .col(alias("resource")),
            )
            .check(col("field").ne("title").or(col("value").is_not_null()))
            .check(
                col("provider")
                    .ne("")
                    .and(col("resource").ne(""))
                    .and(col("value").is_not_null()),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        Table::create()
            .table(alias("music_metadata_release_identity"))
            .col(text("album_id").primary_key().not_null())
            .col(text("musicbrainz_id").not_null())
            .col(text("edition_uuid").not_null())
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_release_identity"), alias("album_id"))
                    .to(alias("music_metadata_albums"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        // The browse projection is refreshed with the completed edition assignment.
        Table::create()
            .table(alias("music_metadata_album_summaries"))
            .col(text("album_id").primary_key().not_null())
            .col(text("title").not_null())
            .col(text("version"))
            .col(text("catalog_number"))
            .col(text("release_date"))
            .col(text("release_type").check(col("release_type").is_in([
                "album",
                "ep",
                "single",
                "broadcast",
                "other",
            ])))
            .col(
                integer("track_count")
                    .check(col("track_count").gte(0))
                    .not_null(),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_album_summaries"), alias("album_id"))
                    .to(alias("music_metadata_albums"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        Table::create()
            .table(alias("music_metadata_dirty_summaries"))
            .col(text("album_id").primary_key().not_null())
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_dirty_summaries"), alias("album_id"))
                    .to(alias("music_metadata_albums"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        Table::create()
            .table(alias("music_metadata_discs"))
            .col(text("id").primary_key().not_null())
            .col(text("album_id").not_null())
            .col(
                integer("disc_number")
                    .check(col("disc_number").gt(0))
                    .not_null(),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_discs"), alias("album_id"))
                    .to(alias("music_metadata_albums"), alias("id")),
            )
            .index(
                Index::create()
                    .unique()
                    .col(alias("album_id"))
                    .col(alias("disc_number")),
            )
            .to_owned(),
        // A recording can be shared by tracks from several editions and providers.
        Table::create()
            .table(alias("music_metadata_recordings"))
            .col(text("id").primary_key().not_null())
            .col(text("musicbrainz_id"))
            .col(text("isrc"))
            .col(text("legacy_isrc").unique_key())
            .extra("WITHOUT ROWID")
            .to_owned(),
        Table::create()
            .table(alias("music_metadata_recording_sources"))
            .col(text("provider").not_null())
            .col(text("resource_id").not_null())
            .col(text("recording_id").not_null())
            .col(text("recording_mbid"))
            .col(text("recording_isrc"))
            .col(text("codec"))
            .col(integer("is_lossless"))
            .col(integer("sample_rate"))
            .col(integer("bits_per_sample"))
            .col(integer("bitrate"))
            .col(
                integer("available")
                    .check(col("available").is_in([0, 1]))
                    .default(1)
                    .not_null(),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(
                        alias("music_metadata_recording_sources"),
                        alias("recording_id"),
                    )
                    .to(alias("music_metadata_recordings"), alias("id")),
            )
            .primary_key(
                Index::create()
                    .col(alias("provider"))
                    .col(alias("resource_id")),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        // Track properties and parent membership are directly queryable.
        Table::create()
            .table(alias("music_metadata_audio"))
            .col(integer("file_id").primary_key())
            .col(
                integer("portable")
                    .not_null()
                    .default(0)
                    .check(col("portable").is_in([0, 1])),
            )
            .col(text("id").not_null().unique_key())
            .col(text("disc_id").not_null())
            .col(text("recording_id").not_null())
            .col(text("recording_mbid"))
            .col(text("recording_isrc"))
            .col(text("recording_content_hash"))
            .col(integer("recording_content_size"))
            .col(text("release_track_mbid"))
            .col(text("confirmed_release_id"))
            .col(text("title").not_null())
            .col(text("track_title_key").not_null())
            .col(integer("track_number"))
            .col(integer("track_total"))
            .col(integer("disc_number"))
            .col(integer("disc_total"))
            .col(integer("duration_ms"))
            .col(text("import_date").not_null())
            .col(text("subtitle"))
            .col(real("bpm"))
            .col(integer("movement_number"))
            .col(integer("movement_total"))
            .col(text("file_path"))
            .col(integer("file_size_bytes"))
            .col(text("file_modified_at"))
            .col(text("format"))
            .col(text("codec"))
            .col(integer("sample_rate"))
            .col(integer("bits_per_sample"))
            .col(integer("channels"))
            .col(text("channel_layout"))
            .col(integer("bitrate"))
            .col(integer("frame_count"))
            .col(text("encoder"))
            .col(text("encoder_settings"))
            .col(text("encoding_date"))
            .col(text("tagging_date"))
            .col(real("replaygain_track_gain_db"))
            .col(real("replaygain_track_peak"))
            .col(real("replaygain_reference_lufs"))
            .col(text("lyrics"))
            .col(text("synced_lyrics"))
            .col(text("original_album_title"))
            .col(text("isrc"))
            .col(text("content_type"))
            .col(text("release_date"))
            .col(text("original_release_date"))
            .col(text("version"))
            .col(text("sort_title"))
            .col(text("grouping"))
            .col(text("description"))
            .col(text("comment"))
            .col(text("copyright"))
            .col(text("license"))
            .col(text("script"))
            .col(text("work"))
            .col(text("part"))
            .col(text("section"))
            .col(text("opus"))
            .col(text("work_id"))
            .col(text("recording_date"))
            .col(text("recording_start_date"))
            .col(text("recording_end_date"))
            .col(text("recording_location"))
            .col(integer("is_live"))
            .col(integer("is_bootleg"))
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_audio"), alias("file_id"))
                    .to(alias("media_files"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_audio"), alias("disc_id"))
                    .to(alias("music_metadata_discs"), alias("id")),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_audio"), alias("recording_id"))
                    .to(alias("music_metadata_recordings"), alias("id")),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_audio"), alias("confirmed_release_id"))
                    .to(alias("music_metadata_albums"), alias("id"))
                    .on_delete(ForeignKeyAction::SetNull),
            )
            .to_owned(),
        Table::create()
            .table(alias("music_metadata_recording_dirty"))
            .col(text("kind").not_null())
            .col(text("value").not_null())
            .primary_key(Index::create().col(alias("kind")).col(alias("value")))
            .extra("WITHOUT ROWID")
            .to_owned(),
        // Shared track UUIDs resolve edition selections on devices with their own catalog UUIDs.
        Table::create()
            .table(alias("music_metadata_album_track_anchors"))
            .col(text("track_uuid").primary_key().not_null())
            .col(text("album_id").not_null())
            .foreign_key(
                ForeignKey::create()
                    .from(
                        alias("music_metadata_album_track_anchors"),
                        alias("album_id"),
                    )
                    .to(alias("music_metadata_albums"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        // A position on a release can link to multiple encodings or a range in a disc image.
        Table::create()
            .table(alias("music_metadata_release_tracks"))
            .col(text("id").primary_key().not_null())
            .col(text("disc_id").not_null())
            .col(text("recording_id").not_null())
            .col(integer("track_number").check(col("track_number").gte(0)))
            .col(text("title").not_null())
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_release_tracks"), alias("disc_id"))
                    .to(alias("music_metadata_discs"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(
                        alias("music_metadata_release_tracks"),
                        alias("recording_id"),
                    )
                    .to(alias("music_metadata_recordings"), alias("id")),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        Table::create()
            .table(alias("music_metadata_dirty_tracks"))
            .col(text("disc_id").primary_key().not_null())
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_dirty_tracks"), alias("disc_id"))
                    .to(alias("music_metadata_discs"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        Table::create()
            .table(alias("music_metadata_audio_layouts"))
            .col(integer("file_id").primary_key())
            .col(
                text("kind")
                    .check(col("kind").is_in(["manual", "cue"]))
                    .not_null(),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_audio_layouts"), alias("file_id"))
                    .to(alias("music_metadata_audio"), alias("file_id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .to_owned(),
        Table::create()
            .table(alias("music_metadata_track_assets"))
            .col(text("track_id").not_null())
            .col(integer("file_id").not_null())
            .col(integer("start_ms").check(col("start_ms").gte(0)).not_null())
            .col(integer("end_ms").check(col("end_ms").gt(col("start_ms"))))
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_track_assets"), alias("track_id"))
                    .to(alias("music_metadata_release_tracks"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_track_assets"), alias("file_id"))
                    .to(alias("music_metadata_audio"), alias("file_id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .primary_key(
                Index::create()
                    .col(alias("track_id"))
                    .col(alias("file_id"))
                    .col(alias("start_ms")),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        // A sparse local source can mask a field without modifying lower-priority evidence.
        Table::create()
            .table(alias("music_metadata_local_overlays"))
            .col(integer("id").primary_key())
            .col(text("album_id"))
            .col(text("disc_id"))
            .col(text("audio_id"))
            .col(text("track_id"))
            .col(
                text("field")
                    .check(col("field").is_in([
                        "title",
                        "version",
                        "catalog_number",
                        "release_date",
                        "original_release_date",
                        "labels",
                        "release_type",
                        "barcode",
                        "release_country",
                        "description",
                        "comment",
                        "sort_title",
                        "subtitle",
                    ]))
                    .not_null(),
            )
            .col(text("value"))
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_local_overlays"), alias("album_id"))
                    .to(alias("music_metadata_albums"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_local_overlays"), alias("disc_id"))
                    .to(alias("music_metadata_discs"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_local_overlays"), alias("audio_id"))
                    .to(alias("music_metadata_audio"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_local_overlays"), alias("track_id"))
                    .to(alias("music_metadata_release_tracks"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .check(
                col("album_id")
                    .is_not_null()
                    .add(col("disc_id").is_not_null())
                    .add(col("audio_id").is_not_null())
                    .add(col("track_id").is_not_null().eq(1)),
            )
            .check(
                col("field")
                    .ne("title")
                    .or(col("value").is_not_null())
                    .or(col("disc_id").is_not_null()),
            )
            .index(
                Index::create()
                    .unique()
                    .col(alias("album_id"))
                    .col(alias("field")),
            )
            .index(
                Index::create()
                    .unique()
                    .col(alias("disc_id"))
                    .col(alias("field")),
            )
            .index(
                Index::create()
                    .unique()
                    .col(alias("audio_id"))
                    .col(alias("field")),
            )
            .index(
                Index::create()
                    .unique()
                    .col(alias("track_id"))
                    .col(alias("field")),
            )
            .to_owned(),
        // Portable name identities and identified artists share one catalog.
        Table::create()
            .table(alias("music_metadata_artists"))
            .col(integer("id").primary_key().auto_increment())
            .col(text("source_uuid").not_null())
            .col(text("name").not_null())
            .col(text("group").not_null().default("#"))
            .col(
                integer("portable")
                    .not_null()
                    .default(1)
                    .check(col("portable").is_in([0, 1])),
            )
            .col(text("musicbrainz_id").unique_key())
            .to_owned(),
        // Keep each track's scalar evidence so removals and rescans can resolve the next candidate.
        Table::create()
            .table(alias("music_metadata_album_evidence"))
            .col(integer("file_id").primary_key())
            .col(text("title").not_null())
            .col(text("tag_title"))
            .col(integer("title_priority").not_null())
            .col(integer("version_priority").not_null())
            .col(text("barcode"))
            .col(text("catalog_number"))
            .col(text("release_country"))
            .col(text("release_type").check(col("release_type").is_in([
                "album",
                "ep",
                "single",
                "broadcast",
                "other",
            ])))
            .col(text("release_status"))
            .col(integer("disc_count"))
            .col(integer("declared_track_count"))
            .col(real("replaygain_album_gain_db"))
            .col(real("replaygain_album_peak"))
            .col(real("replaygain_reference_lufs"))
            .col(text("release_date"))
            .col(text("original_release_date"))
            .col(text("version"))
            .col(text("sort_title"))
            .col(text("grouping"))
            .col(text("description"))
            .col(text("comment"))
            .col(text("copyright"))
            .col(text("license"))
            .col(text("script"))
            .col(text("work"))
            .col(text("part"))
            .col(text("section"))
            .col(text("opus"))
            .col(text("work_id"))
            .col(text("recording_date"))
            .col(text("recording_start_date"))
            .col(text("recording_end_date"))
            .col(text("recording_location"))
            .col(integer("is_live"))
            .col(integer("is_bootleg"))
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_album_evidence"), alias("file_id"))
                    .to(alias("music_metadata_audio"), alias("file_id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .to_owned(),
        Table::create()
            .table(alias("music_metadata_disc_evidence"))
            .col(integer("file_id").primary_key())
            .col(text("title"))
            .col(integer("title_priority").not_null())
            .col(text("media_format"))
            .col(text("directory_path"))
            .col(integer("declared_track_count"))
            .col(text("release_date"))
            .col(text("original_release_date"))
            .col(text("version"))
            .col(text("sort_title"))
            .col(text("grouping"))
            .col(text("description"))
            .col(text("comment"))
            .col(text("copyright"))
            .col(text("license"))
            .col(text("script"))
            .col(text("work"))
            .col(text("part"))
            .col(text("section"))
            .col(text("opus"))
            .col(text("work_id"))
            .col(text("recording_date"))
            .col(text("recording_start_date"))
            .col(text("recording_end_date"))
            .col(text("recording_location"))
            .col(integer("is_live"))
            .col(integer("is_bootleg"))
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_disc_evidence"), alias("file_id"))
                    .to(alias("music_metadata_audio"), alias("file_id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .to_owned(),
        // Track contributions maintain the distinct release keys used for subsequent matching.
        Table::create()
            .table(alias("music_metadata_track_release_keys"))
            .col(integer("file_id").not_null())
            .col(text("album_id").not_null())
            .col(text("field").not_null())
            .col(text("value").not_null())
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_track_release_keys"), alias("file_id"))
                    .to(alias("music_metadata_audio"), alias("file_id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(
                        alias("music_metadata_track_release_keys"),
                        alias("album_id"),
                    )
                    .to(alias("music_metadata_albums"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .primary_key(Index::create().col(alias("file_id")).col(alias("field")))
            .extra("WITHOUT ROWID")
            .to_owned(),
        Table::create()
            .table(alias("music_metadata_release_keys"))
            .col(text("album_id").not_null())
            .col(text("field").not_null())
            .col(text("value").not_null())
            .col(integer("uses").check(col("uses").gt(0)).not_null())
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_release_keys"), alias("album_id"))
                    .to(alias("music_metadata_albums"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .primary_key(
                Index::create()
                    .col(alias("album_id"))
                    .col(alias("field"))
                    .col(alias("value")),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        // A committed scan can be reconciled later even if its caller is interrupted.
        Table::create()
            .table(alias("music_metadata_dirty_directories"))
            .col(text("directory_scope").primary_key().not_null())
            .extra("WITHOUT ROWID")
            .to_owned(),
        // Artist associations are optional so a scan can keep a credit before disambiguation.
        Table::create()
            .table(alias("music_metadata_credits"))
            .col(integer("file_id").not_null())
            .col(
                integer("scope")
                    .check(col("scope").between(0, 2))
                    .not_null(),
            )
            .col(integer("position").check(col("position").gte(0)).not_null())
            .col(text("credited_as").not_null())
            .col(text("role").not_null())
            .col(text("artist_id"))
            .col(
                integer("artist_confirmed")
                    .check(col("artist_confirmed").is_in([0, 1]))
                    .default(0)
                    .not_null(),
            )
            .col(text("instrument"))
            .col(text("source_kind").not_null())
            .col(text("source_provider").not_null())
            .col(text("source_key"))
            .col(text("source_path"))
            .col(text("source_pattern"))
            .col(text("source_entity_id"))
            .col(real("source_confidence"))
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_credits"), alias("file_id"))
                    .to(alias("music_metadata_audio"), alias("file_id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_credits"), alias("artist_id"))
                    .to(alias("music_metadata_artists"), alias("source_uuid"))
                    .on_delete(ForeignKeyAction::SetNull),
            )
            .primary_key(
                Index::create()
                    .col(alias("file_id"))
                    .col(alias("source_kind"))
                    .col(alias("source_provider"))
                    .col(alias("scope"))
                    .col(alias("position")),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        // Recording-level credits can exist before a particular file or edition is present.
        Table::create()
            .table(alias("music_metadata_recording_credits"))
            .col(text("recording_id").not_null())
            .col(integer("position").check(col("position").gte(0)).not_null())
            .col(text("source_kind").not_null())
            .col(text("source_provider").not_null())
            .col(text("credited_as").not_null())
            .col(text("role").not_null())
            .col(text("artist_id"))
            .col(text("instrument"))
            .col(text("source_key"))
            .col(text("source_path"))
            .col(text("source_pattern"))
            .col(text("source_entity_id"))
            .col(real("source_confidence"))
            .foreign_key(
                ForeignKey::create()
                    .from(
                        alias("music_metadata_recording_credits"),
                        alias("recording_id"),
                    )
                    .to(alias("music_metadata_recordings"), alias("id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .foreign_key(
                ForeignKey::create()
                    .from(
                        alias("music_metadata_recording_credits"),
                        alias("artist_id"),
                    )
                    .to(alias("music_metadata_artists"), alias("source_uuid"))
                    .on_delete(ForeignKeyAction::SetNull),
            )
            .primary_key(
                Index::create()
                    .col(alias("recording_id"))
                    .col(alias("source_kind"))
                    .col(alias("source_provider"))
                    .col(alias("position")),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        // Manual decisions are replicated separately from local parser evidence.
        Table::create()
            .table(alias("music_metadata_sync_edits"))
            .col(integer("id").primary_key().auto_increment())
            .col(text("source_uuid").not_null().unique_key())
            .col(
                text("kind")
                    .check(col("kind").is_in(["album", "artist", "anchor"]))
                    .not_null(),
            )
            .col(text("owner_uuid").not_null())
            .col(text("edition_uuid"))
            .col(text("target_uuid"))
            .col(text("label"))
            .col(
                integer("generation")
                    .check(col("generation").gte(0))
                    .default(0)
                    .not_null(),
            )
            .to_owned(),
        // Reserve negative browse IDs for richer identities while positive IDs stay compatible.
        Table::create()
            .table(alias("music_metadata_browse_handles"))
            .col(
                integer("id")
                    .check(col("id").lte(2147483647))
                    .primary_key()
                    .auto_increment(),
            )
            .col(
                text("kind")
                    .check(col("kind").is_in(["album", "artist"]))
                    .not_null(),
            )
            .col(text("source_uuid").not_null())
            .index(
                Index::create()
                    .unique()
                    .col(alias("kind"))
                    .col(alias("source_uuid")),
            )
            .to_owned(),
        // Identifiers and other repeatable text tags retain their input order and scope.
        Table::create()
            .table(alias("music_metadata_values"))
            .col(integer("file_id").not_null())
            .col(
                integer("scope")
                    .check(col("scope").between(0, 2))
                    .not_null(),
            )
            .col(text("field").not_null())
            .col(text("qualifier").not_null())
            .col(integer("position").check(col("position").gte(0)).not_null())
            .col(text("value").not_null())
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_values"), alias("file_id"))
                    .to(alias("music_metadata_audio"), alias("file_id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .primary_key(
                Index::create()
                    .col(alias("file_id"))
                    .col(alias("scope"))
                    .col(alias("field"))
                    .col(alias("qualifier"))
                    .col(alias("position")),
            )
            .extra("WITHOUT ROWID")
            .to_owned(),
        // Structured details are loaded only in full metadata queries.
        Table::create()
            .table(alias("music_metadata_details"))
            .col(integer("file_id").not_null())
            .col(
                integer("scope")
                    .check(col("scope").between(0, 2))
                    .not_null(),
            )
            .col(text("artwork"))
            .col(text("links"))
            .col(text("provenance"))
            .foreign_key(
                ForeignKey::create()
                    .from(alias("music_metadata_details"), alias("file_id"))
                    .to(alias("music_metadata_audio"), alias("file_id"))
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .primary_key(Index::create().col(alias("file_id")).col(alias("scope")))
            .extra("WITHOUT ROWID")
            .to_owned(),
    ]
}

fn indexes() -> [IndexCreateStatement; 29] {
    [
        Index::create()
            .name("music_metadata_album_uuid")
            .table(alias("music_metadata_albums"))
            .unique()
            .col(alias("id"))
            .to_owned(),
        Index::create()
            .name("idx_music_metadata_artists_source_uuid")
            .table(alias("music_metadata_artists"))
            .col(alias("source_uuid"))
            .unique()
            .to_owned(),
        Index::create()
            .name("music_metadata_album_directory")
            .table(alias("music_metadata_albums"))
            .col(alias("directory_scope"))
            .col(alias("id"))
            .to_owned(),
        Index::create()
            .name("music_metadata_album_group")
            .table(alias("music_metadata_albums"))
            .col(alias("group_id"))
            .and_where(col("group_id").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_file_digest")
            .table(alias("media_files"))
            .col(alias("content_hash"))
            .col(alias("content_size"))
            .and_where(col("content_hash").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_album_field_approved")
            .table(alias("music_metadata_album_field_values"))
            .unique()
            .col(alias("album_id"))
            .col(alias("field"))
            .and_where(col("decision").eq(1).and(col("provider").ne("")))
            .to_owned(),
        Index::create()
            .name("music_metadata_release_identity_mbid")
            .table(alias("music_metadata_release_identity"))
            .col(alias("musicbrainz_id"))
            .to_owned(),
        Index::create()
            .name("music_metadata_release_identity_edition")
            .table(alias("music_metadata_release_identity"))
            .col(alias("edition_uuid"))
            .to_owned(),
        Index::create()
            .name("music_metadata_album_summary_type")
            .table(alias("music_metadata_album_summaries"))
            .col(alias("release_type"))
            .col(alias("album_id"))
            .and_where(col("release_type").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_recording_mbid")
            .table(alias("music_metadata_recordings"))
            .col(alias("musicbrainz_id"))
            .and_where(col("musicbrainz_id").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_recording_isrc")
            .table(alias("music_metadata_recordings"))
            .col(alias("isrc"))
            .and_where(col("isrc").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_recording_sources_recording")
            .table(alias("music_metadata_recording_sources"))
            .col(alias("recording_id"))
            .to_owned(),
        Index::create()
            .name("music_metadata_audio_confirmed_release")
            .table(alias("music_metadata_audio"))
            .col(alias("confirmed_release_id"))
            .and_where(col("confirmed_release_id").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_audio_recording")
            .table(alias("music_metadata_audio"))
            .col(alias("recording_id"))
            .to_owned(),
        Index::create()
            .name("music_metadata_audio_release_track")
            .table(alias("music_metadata_audio"))
            .col(alias("release_track_mbid"))
            .and_where(col("release_track_mbid").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_audio_recording_mbid")
            .table(alias("music_metadata_audio"))
            .col(alias("recording_mbid"))
            .and_where(col("recording_mbid").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_audio_recording_isrc")
            .table(alias("music_metadata_audio"))
            .col(alias("recording_isrc"))
            .and_where(col("recording_isrc").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_source_recording_mbid")
            .table(alias("music_metadata_recording_sources"))
            .col(alias("recording_mbid"))
            .and_where(col("recording_mbid").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_source_recording_isrc")
            .table(alias("music_metadata_recording_sources"))
            .col(alias("recording_isrc"))
            .and_where(col("recording_isrc").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_album_track_anchors_album")
            .table(alias("music_metadata_album_track_anchors"))
            .col(alias("album_id"))
            .col(alias("track_uuid"))
            .to_owned(),
        Index::create()
            .name("music_metadata_release_tracks_disc")
            .table(alias("music_metadata_release_tracks"))
            .col(alias("disc_id"))
            .col(alias("track_number"))
            .col(alias("id"))
            .to_owned(),
        Index::create()
            .name("music_metadata_release_tracks_recording")
            .table(alias("music_metadata_release_tracks"))
            .col(alias("recording_id"))
            .to_owned(),
        Index::create()
            .name("music_metadata_track_assets_file")
            .table(alias("music_metadata_track_assets"))
            .col(alias("file_id"))
            .col(alias("start_ms"))
            .col(alias("track_id"))
            .to_owned(),
        Index::create()
            .name("music_metadata_track_release_keys_album")
            .table(alias("music_metadata_track_release_keys"))
            .col(alias("album_id"))
            .to_owned(),
        Index::create()
            .name("music_metadata_credits_artist")
            .table(alias("music_metadata_credits"))
            .col(alias("artist_id"))
            .and_where(col("artist_id").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_recording_credits_artist")
            .table(alias("music_metadata_recording_credits"))
            .col(alias("artist_id"))
            .and_where(col("artist_id").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_sync_edit_owner")
            .table(alias("music_metadata_sync_edits"))
            .col(alias("kind"))
            .col(alias("owner_uuid"))
            .col(alias("edition_uuid"))
            .to_owned(),
        Index::create()
            .name("music_metadata_sync_edit_edition")
            .table(alias("music_metadata_sync_edits"))
            .col(alias("edition_uuid"))
            .and_where(col("edition_uuid").is_not_null())
            .to_owned(),
        Index::create()
            .name("music_metadata_sync_edit_target")
            .table(alias("music_metadata_sync_edits"))
            .col(alias("kind"))
            .col(alias("target_uuid"))
            .to_owned(),
    ]
}
