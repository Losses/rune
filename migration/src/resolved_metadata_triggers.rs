//! Capture the evidence SQLite removes through foreign-key cascades.

use sea_orm_migration::{
    prelude::*,
    sea_orm::ConnectionTrait,
    sea_query::{OnConflict, SqliteQueryBuilder},
};

pub(super) async fn install(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let disc = Alias::new("disc_id");
    let previous = Expr::col((Alias::new("OLD"), disc.clone()));
    let discs = Query::select()
        .column(Alias::new("id"))
        .from(Alias::new("music_metadata_discs"))
        .and_where(Expr::col(Alias::new("id")).eq(previous))
        .to_owned();
    let mut dirty_disc = Query::insert();
    dirty_disc
        .into_table(Alias::new("music_metadata_dirty_tracks"))
        .columns([disc])
        .select_from(discs)
        .map_err(|error| DbErr::Custom(error.to_string()))?
        .on_conflict(OnConflict::new().do_nothing().to_owned());
    let mut audio = vec![dirty_disc.to_string(SqliteQueryBuilder)];

    // A generic sync tombstone can cascade before Rust can read the former disc or identifiers.
    // The trigger only records those keys. Rust reconciles tracks and cleans up the parents.
    audio.extend(recording_keys()?);
    // SeaQuery builds the queue writes but has no CREATE TRIGGER declaration.
    let body = audio.join("; ");
    manager.get_connection().execute_unprepared(&format!(
        "CREATE TRIGGER music_metadata_audio_deleted AFTER DELETE ON music_metadata_audio BEGIN {body}; END"
    )).await?;
    Ok(())
}

pub(super) async fn uninstall(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // SeaQuery has no DROP TRIGGER builder. Remove the hook before its queue tables.
    manager
        .get_connection()
        .execute_unprepared("DROP TRIGGER music_metadata_audio_deleted")
        .await?;
    Ok(())
}

fn recording_keys() -> Result<Vec<String>, DbErr> {
    let mut statements = Vec::new();
    for (kind, name) in [
        ("recording", "recording_id"),
        ("mbid", "recording_mbid"),
        ("isrc", "recording_isrc"),
    ] {
        let before = Expr::col((Alias::new("OLD"), Alias::new(name)));
        let keys = Query::select()
            .exprs([Expr::val(kind), before.clone()])
            .and_where(before.is_not_null())
            .to_owned();
        let mut insert = Query::insert();
        insert
            .into_table(Alias::new("music_metadata_recording_dirty"))
            .columns([Alias::new("kind"), Alias::new("value")])
            .select_from(keys)
            .map_err(|error| DbErr::Custom(error.to_string()))?
            .on_conflict(OnConflict::new().do_nothing().to_owned());
        statements.push(insert.to_string(SqliteQueryBuilder));
    }
    Ok(statements)
}
