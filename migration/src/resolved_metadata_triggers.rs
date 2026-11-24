//! SQLite lifecycle hooks for row changes, including foreign-key cascades.

use std::slice;

use sea_orm_migration::{
    prelude::*,
    sea_orm::ConnectionTrait,
    sea_query::{
        BinOper, InsertStatement, OnConflict, QueryStatementWriter, SelectStatement,
        SqliteQueryBuilder,
    },
};

type Trigger = (
    &'static str,
    &'static str,
    &'static str,
    Option<Expr>,
    Vec<String>,
);

pub(super) async fn install(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for (name, event, table, guard, statements) in triggers()? {
        // SQLite has no Rust callback for cascading writes. SeaQuery builds the
        // body and guard but has no CREATE TRIGGER declaration.
        let guard = guard.map_or_else(String::new, |guard| {
            let test = Query::select()
                .expr(Expr::val(1))
                .and_where(guard)
                .to_owned();
            format!("WHEN EXISTS ({}) ", sql(&test))
        });
        let body = statements.join("; ");
        manager
            .get_connection()
            .execute_unprepared(&format!(
                "CREATE TRIGGER music_metadata_{name} {event} ON {table} {guard}BEGIN {body}; END"
            ))
            .await?;
    }
    Ok(())
}

pub(super) async fn uninstall(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // SeaQuery has no DROP TRIGGER statement.
    for (name, ..) in triggers()? {
        manager
            .get_connection()
            .execute_unprepared(&format!("DROP TRIGGER music_metadata_{name}"))
            .await?;
    }
    Ok(())
}

fn alias(name: &str) -> Alias {
    Alias::new(name)
}

fn column(name: &str) -> Expr {
    Expr::col(alias(name))
}

fn row(side: &str, name: &str) -> Expr {
    Expr::col((alias(side), alias(name)))
}

fn changed(name: &str) -> Expr {
    row("OLD", name).binary(BinOper::IsNot, row("NEW", name))
}

fn sql(statement: &impl QueryStatementWriter) -> String {
    statement.to_string(SqliteQueryBuilder)
}

fn insert(table: &str, columns: &[&str]) -> InsertStatement {
    Query::insert()
        .into_table(alias(table))
        .columns(columns.iter().map(|name| alias(name)))
        .to_owned()
}

fn ignore() -> OnConflict {
    OnConflict::new().do_nothing().to_owned()
}

fn insert_select(
    mut statement: InsertStatement,
    query: SelectStatement,
) -> Result<InsertStatement, DbErr> {
    statement
        .select_from(query)
        .map_err(|error| DbErr::Custom(error.to_string()))?;
    Ok(statement)
}

fn select(table: &str, key: &str, value: Expr) -> SelectStatement {
    Query::select()
        .from(alias(table))
        .and_where(column(key).eq(value))
        .to_owned()
}

fn missing(table: &str, key: &str, value: Expr) -> Expr {
    Expr::not_exists(select(table, key, value).expr(Expr::val(1)).to_owned())
}

fn disc_summary(discs: &[Expr]) -> Result<String, DbErr> {
    let evidence = Query::select()
        .column(alias("album_id"))
        .from(alias("music_metadata_discs"))
        .and_where(column("id").is_in(discs.iter().cloned()))
        .to_owned();
    let mut dirty = insert_select(
        insert("music_metadata_dirty_summaries", &["album_id"]),
        evidence,
    )?;
    Ok(sql(dirty.on_conflict(ignore())))
}

fn recording_dirty() -> Result<Vec<Trigger>, DbErr> {
    // A changed media file can invalidate evidence without replacing its audio row.
    let mut changed_file = Vec::new();
    let mut deleted_evidence = Vec::new();
    for (kind, field) in [
        ("mbid", "recording_mbid"),
        ("isrc", "recording_isrc"),
        ("recording", "recording_id"),
    ] {
        let file = select("music_metadata_audio", "file_id", row("NEW", "id"))
            .exprs([Expr::val(kind), column(field)])
            .and_where(column(field).is_not_null())
            .to_owned();
        let deleted = Query::select()
            .exprs([Expr::val(kind), row("OLD", field)])
            .and_where(row("OLD", field).is_not_null())
            .to_owned();
        for (statements, source) in [(&mut changed_file, file), (&mut deleted_evidence, deleted)] {
            let mut dirty = insert_select(
                insert("music_metadata_recording_dirty", &["kind", "value"]),
                source,
            )?;
            statements.push(sql(dirty.on_conflict(ignore())));
        }
    }

    // Foreign-key deletion has to retain the old identifiers before reconciliation.
    Ok(vec![
        (
            "recording_content_update",
            "AFTER UPDATE OF content_hash, content_size",
            "media_files",
            Some(changed("content_hash").or(changed("content_size"))),
            changed_file,
        ),
        (
            "recording_0_DELETE",
            "AFTER DELETE",
            "music_metadata_audio",
            None,
            deleted_evidence.clone(),
        ),
        (
            "recording_1_DELETE",
            "AFTER DELETE",
            "music_metadata_recording_sources",
            None,
            deleted_evidence,
        ),
    ])
}

fn tracks_and_summaries() -> Result<Vec<Trigger>, DbErr> {
    let previous = row("OLD", "disc_id");
    let current = row("NEW", "disc_id");
    let dirty_tracks = |args: &[Expr]| -> Result<String, DbErr> {
        // Moving the last audio may already have removed its old disc.
        let discs = Query::select()
            .column(alias("id"))
            .from(alias("music_metadata_discs"))
            .and_where(column("id").is_in(args.iter().cloned()))
            .to_owned();
        let mut dirty = insert_select(insert("music_metadata_dirty_tracks", &["disc_id"]), discs)?;
        Ok(sql(dirty.on_conflict(ignore())))
    };
    let count_change = changed("disc_id").or(changed("track_number"));
    let track_change = count_change
        .clone()
        .or(changed("title"))
        .or(changed("track_title_key"))
        .or(changed("release_track_mbid"));

    // Invalidate logical tracks and summaries when their source changes or disappears.
    let mut triggers = vec![
        (
            "tracks_audio_insert",
            "AFTER INSERT",
            "music_metadata_audio",
            None,
            vec![dirty_tracks(slice::from_ref(&current))?],
        ),
        (
            "tracks_audio_update",
            "AFTER UPDATE OF disc_id, track_number, track_title_key, title, release_track_mbid",
            "music_metadata_audio",
            Some(track_change),
            vec![dirty_tracks(&[previous.clone(), current.clone()])?],
        ),
        (
            "tracks_audio_delete",
            "BEFORE DELETE",
            "music_metadata_audio",
            None,
            vec![dirty_tracks(slice::from_ref(&previous))?],
        ),
        (
            "summary_track_insert",
            "AFTER INSERT",
            "music_metadata_release_tracks",
            None,
            vec![disc_summary(slice::from_ref(&current))?],
        ),
        (
            "summary_track_delete",
            "BEFORE DELETE",
            "music_metadata_release_tracks",
            None,
            vec![disc_summary(slice::from_ref(&previous))?],
        ),
        (
            "summary_track_move",
            "BEFORE UPDATE OF disc_id",
            "music_metadata_release_tracks",
            Some(changed("disc_id")),
            vec![disc_summary(&[previous.clone(), current.clone()])?],
        ),
        (
            "summary_audio_move",
            "BEFORE UPDATE OF disc_id, track_number",
            "music_metadata_audio",
            Some(count_change),
            vec![disc_summary(&[previous.clone(), current])?],
        ),
        (
            "summary_audio_delete",
            "BEFORE DELETE",
            "music_metadata_audio",
            None,
            vec![disc_summary(&[previous])?],
        ),
    ];

    // Evidence can be replaced independently of its audio parent.
    for (name, event, side) in [
        ("summary_evidence_insert", "AFTER INSERT", "NEW"),
        ("summary_evidence_update", "AFTER UPDATE", "NEW"),
        ("summary_evidence_delete", "AFTER DELETE", "OLD"),
    ] {
        let album = Query::select()
            .column((alias("disc"), alias("album_id")))
            .from_as(alias("music_metadata_audio"), alias("audio"))
            .join_as(
                JoinType::InnerJoin,
                alias("music_metadata_discs"),
                alias("disc"),
                row("disc", "id").eq(row("audio", "disc_id")),
            )
            .and_where(row("audio", "file_id").eq(row(side, "file_id")))
            .to_owned();
        let mut dirty = insert_select(
            insert("music_metadata_dirty_summaries", &["album_id"]),
            album,
        )?;
        triggers.push((
            name,
            event,
            "music_metadata_album_evidence",
            None,
            vec![sql(dirty.on_conflict(ignore()))],
        ));
    }
    Ok(triggers)
}

fn anchors_and_cleanup() -> Result<Vec<Trigger>, DbErr> {
    // Stable anchors follow an audio file after edition regrouping.
    let membership = select("music_metadata_discs", "id", row("NEW", "disc_id"))
        .expr(row("NEW", "id"))
        .column(alias("album_id"))
        .to_owned();
    let mut anchor = insert_select(
        insert(
            "music_metadata_album_track_anchors",
            &["track_uuid", "album_id"],
        ),
        membership,
    )?;
    let anchor = sql(anchor.on_conflict(
        OnConflict::column(alias("track_uuid"))
            .update_column(alias("album_id"))
            .action_and_where(column("album_id").ne(row("excluded", "album_id")))
            .to_owned(),
    ));

    // A cascading delete or a move can remove the last child of each parent.
    let disc = row("OLD", "disc_id");
    let remove_disc = sql(Query::delete()
        .from_table(alias("music_metadata_discs"))
        .and_where(column("id").eq(disc.clone()))
        .and_where(missing("music_metadata_audio", "disc_id", disc.clone())));
    let album = row("OLD", "album_id");
    let remove_album = sql(Query::delete()
        .from_table(alias("music_metadata_albums"))
        .and_where(column("id").eq(album.clone()))
        .and_where(column("portable_title").is_null())
        .and_where(column("group_id").is_null())
        .and_where(missing(
            "music_metadata_album_field_values",
            "album_id",
            album.clone(),
        ))
        .and_where(missing(
            "music_metadata_local_overlays",
            "album_id",
            album.clone(),
        ))
        .and_where(missing("music_metadata_discs", "album_id", album)));
    let track = row("OLD", "track_id");
    let remove_track = sql(Query::delete()
        .from_table(alias("music_metadata_release_tracks"))
        .and_where(column("id").eq(track.clone()))
        .and_where(missing("music_metadata_track_assets", "track_id", track)));

    // Reconcile the old directory even when deleting its last indexed file.
    let directory = Query::select()
        .column((alias("album"), alias("directory_scope")))
        .from_as(alias("music_metadata_discs"), alias("disc"))
        .join_as(
            JoinType::InnerJoin,
            alias("music_metadata_albums"),
            alias("album"),
            row("album", "id").eq(row("disc", "album_id")),
        )
        .and_where(row("disc", "id").eq(disc))
        .to_owned();
    let mut queue = insert_select(
        insert("music_metadata_dirty_directories", &["directory_scope"]),
        directory,
    )?;
    Ok(vec![
        (
            "album_anchor_insert",
            "AFTER INSERT",
            "music_metadata_audio",
            None,
            vec![anchor.clone()],
        ),
        (
            "album_anchor_move",
            "AFTER UPDATE OF disc_id",
            "music_metadata_audio",
            None,
            vec![anchor],
        ),
        (
            "audio_delete",
            "AFTER DELETE",
            "music_metadata_audio",
            None,
            vec![remove_disc.clone()],
        ),
        (
            "audio_move",
            "AFTER UPDATE OF disc_id",
            "music_metadata_audio",
            None,
            vec![remove_disc],
        ),
        (
            "disc_delete",
            "AFTER DELETE",
            "music_metadata_discs",
            None,
            vec![remove_album],
        ),
        (
            "track_asset_deleted",
            "AFTER DELETE",
            "music_metadata_track_assets",
            None,
            vec![remove_track],
        ),
        (
            "queue_audio_delete",
            "BEFORE DELETE",
            "music_metadata_audio",
            None,
            vec![sql(queue.on_conflict(ignore()))],
        ),
    ])
}

fn catalog_handles() -> Vec<Trigger> {
    // Stable browse handles are allocated for new identities in the same write.
    [
        ("browse_album", "album", "music_metadata_albums", "id"),
        (
            "browse_artist",
            "artist",
            "music_metadata_artists",
            "source_uuid",
        ),
    ]
    .into_iter()
    .map(|(name, kind, table, source)| {
        let handle = sql(
            insert("music_metadata_browse_handles", &["kind", "source_uuid"])
                .values_panic([Expr::val(kind), row("NEW", source)])
                .on_conflict(ignore()),
        );
        (name, "AFTER INSERT", table, None, vec![handle])
    })
    .collect()
}

fn release_key_counts() -> Vec<Trigger> {
    // Keep edition matching bounded by distinct keys rather than track count.
    let identity = ["album_id", "field", "value"];
    let increment = sql(insert(
        "music_metadata_release_keys",
        &["album_id", "field", "value", "uses"],
    )
    .values_panic([
        row("NEW", "album_id"),
        row("NEW", "field"),
        row("NEW", "value"),
        Expr::val(1),
    ])
    .on_conflict(
        OnConflict::columns(identity.map(alias))
            .value(alias("uses"), column("uses").add(1))
            .to_owned(),
    ));
    let old = || {
        Cond::all()
            .add(column("album_id").eq(row("OLD", "album_id")))
            .add(column("field").eq(row("OLD", "field")))
            .add(column("value").eq(row("OLD", "value")))
    };

    // Delete the last reference before decrementing the remaining counts.
    let remove = sql(Query::delete()
        .from_table(alias("music_metadata_release_keys"))
        .cond_where(old().add(column("uses").eq(1))));
    let decrement = sql(Query::update()
        .table(alias("music_metadata_release_keys"))
        .value(alias("uses"), column("uses").sub(1))
        .cond_where(old()));
    vec![
        (
            "release_key_insert",
            "AFTER INSERT",
            "music_metadata_track_release_keys",
            None,
            vec![increment],
        ),
        (
            "release_key_delete",
            "AFTER DELETE",
            "music_metadata_track_release_keys",
            None,
            vec![remove, decrement],
        ),
    ]
}

fn triggers() -> Result<Vec<Trigger>, DbErr> {
    let mut definitions = recording_dirty()?;
    definitions.extend(tracks_and_summaries()?);
    definitions.extend(anchors_and_cleanup()?);
    definitions.extend(catalog_handles());
    definitions.extend(release_key_counts());
    Ok(definitions)
}
