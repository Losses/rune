use support::fixtures;

use anyhow::Result;
use database::test_fixtures::causal_library as support;
use database::{
    entities::{media_files, mixes},
    sync::{accepted::AcceptedState, model::*},
};
use migration::{Migrator, MigratorTrait};
use rust_decimal::Decimal;
use sea_orm::{
    ColumnTrait, ConnectionTrait, Database, EntityTrait, PaginatorTrait, QueryFilter, Set,
    TransactionTrait, sea_query::Expr,
};
use support::{SqlLibrary, key, playlist};
use sync_orm::causal_progress;
use uuid::Uuid;

async fn seed(local: &SqlLibrary, invalid: bool) -> Result<()> {
    for number in 1..=257 {
        let identity = if number == 129 {
            String::new()
        } else if invalid && number == 130 {
            "invalid".into()
        } else {
            Uuid::from_u128(number as u128).to_string()
        };
        media_files::Entity::insert(media_files::ActiveModel {
            id: Set(number),
            file_name: Set(format!("file{number}")),
            directory: Set(String::new()),
            extension: Set("ogg".into()),
            file_hash: Set(format!("hash{number}")),
            last_modified: Set("old".into()),
            sample_rate: Set(44100),
            duration: Set(Decimal::ONE),
            source_uuid: Set(identity),
            ..Default::default()
        })
        .exec(&local.db)
        .await?;
    }
    Ok(())
}
async fn initialized(local: &SqlLibrary) -> Result<bool> {
    causal_progress::initialization_complete_on(
        &local.db,
        local.sync_store.record_store().namespace(),
    )
    .await
}
async fn finish_and_check(local: &SqlLibrary) -> Result<()> {
    let row = media_files::Entity::find_by_id(129)
        .one(&local.db)
        .await?
        .unwrap();
    assert!(
        row.source_uuid.is_empty(),
        "failed batch assigned a source UUID"
    );
    assert!(!initialized(local).await?);
    let reopened = fixtures::open_library(
        local.db.clone(),
        local.sync_store.record_store().author(),
        Some(local.sync_store.record_store().namespace()),
    )
    .await?;
    reopened.import_basic().await?;
    let head = reopened.head().await?;
    assert_eq!(
        head.sequence,
        257 + mixes::Entity::find().count(&local.db).await?
    );
    assert!(initialized(local).await?);
    let assigned = media_files::Entity::find_by_id(129)
        .one(&local.db)
        .await?
        .unwrap()
        .source_uuid;
    assert!(Uuid::parse_str(&assigned).is_ok());
    reopened.import_basic().await?;
    assert_eq!(reopened.head().await?, head);
    assert_eq!(
        media_files::Entity::find_by_id(129)
            .one(&local.db)
            .await?
            .unwrap()
            .source_uuid,
        assigned
    );
    Ok(())
}
#[tokio::test]
async fn failure_rolls_back_only_current_source_batch_and_restart_is_idempotent() -> Result<()> {
    let local = SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    seed(&local, true).await?;
    let error = format!("{:#}", local.sync_store.import_basic().await.unwrap_err());
    assert!(error.contains("tracks") && error.contains("130"), "{error}");
    assert_eq!(local.sync_store.head().await?.sequence, 126);
    media_files::Entity::update_many()
        .col_expr(
            media_files::Column::SourceUuid,
            Expr::value(Uuid::from_u128(130).to_string()),
        )
        .filter(media_files::Column::Id.eq(130))
        .exec(&local.db)
        .await?;
    finish_and_check(&local).await
}
#[tokio::test]
async fn accepted_view_pages_are_bounded_and_skip_removed_values() -> Result<()> {
    let local = SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    for number in 1..=3 {
        let key = key(Collection::Playlists, Uuid::from_u128(number));
        local
            .peer
            .write_local(key.clone(), playlist(&number.to_string()))
            .await?;
        if number == 2 {
            local.peer.write_local(key, None).await?;
        }
    }
    let tx = local.db.begin().await?;
    let store = local.sync_store.record_store();
    let view = AcceptedState::new(&tx, &store);
    let mut after = None;
    let mut ids = Vec::new();
    let mut pages = 0;
    loop {
        let page = view
            .page(
                Collection::Playlists,
                after,
                sync::PageLimits {
                    max_scanned: 1,
                    max_bytes: 1024 * 1024,
                },
            )
            .await?;
        assert!(page.records.len() <= 1);
        ids.extend(page.records.iter().map(|r| r.key.id));
        pages += 1;
        if page.done {
            break;
        }
        assert_ne!(page.next, after);
        after = page.next;
    }
    assert_eq!(ids, vec![Uuid::from_u128(1), Uuid::from_u128(3)]);
    assert!(pages <= 3);
    assert!(
        view.get(&key(Collection::Playlists, Uuid::from_u128(2)))
            .await?
            .is_none()
    );
    tx.commit().await?;
    Ok(())
}
#[tokio::test]
async fn legacy_noncanonical_mix_numbers_keep_their_values_through_import() -> Result<()> {
    let actor = Uuid::new_v4();
    let _ = migration::initialize_node_id(actor.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    let before_conversion = Migrator::migrations()
        .iter()
        .position(|m| m.name() == "m20261101_000032_create_sync_library")
        .unwrap() as u32;
    Migrator::up(&db, Some(before_conversion)).await?;
    let legacy = Uuid::from_u128(0xd0);
    let other = Uuid::from_u128(0xc0);
    for sql in [
        format!("INSERT INTO mixes(id,name,\"group\",locked,scriptlet_mode,hlc_uuid) VALUES(900,'Legacy','g',0,0,'{legacy}'),(901,'Other','g',0,0,'{other}')"),
        "INSERT INTO mix_queries(id,mix_id,operator,parameter,\"group\") VALUES(1001,900,'lib::random','005',0),(1002,900,'pipe::recommend','-07',0),(1003,900,'pipe::recommend','+3',0),(1004,900,'pipe::limit','+050',0),(1005,900,'pipe::limit','5x',0),(1006,900,'pipe::limit','-1',0),(1007,900,'lib::all','true',0),(1008,901,'pipe::limit','50',0),(1009,901,'lib::random','-2',0)".into(),
    ] {
        db.execute_unprepared(&sql).await?;
    }
    migration::up_with_actor(&db, Some(&actor.to_string()), None).await?;
    let library = fixtures::open_library(db.clone(), actor, None).await?;
    library.import_basic().await?;
    for (id, expected) in [
        (
            legacy,
            vec![
                ("lib::random", "5"),
                ("pipe::recommend", "-7"),
                ("pipe::recommend", "3"),
                ("pipe::limit", "50"),
                ("lib::all", "true"),
            ],
        ),
        (other, vec![("pipe::limit", "50"), ("lib::random", "-2")]),
    ] {
        let Some(Payload::Mix(mix)) = library
            .get(&key(Collection::Mixes, id))
            .await?
            .unwrap()
            .value
        else {
            panic!("mix {id} was not imported");
        };
        let clauses: Vec<_> = mix
            .queries
            .iter()
            .map(|clause| (clause.operator.as_str(), clause.parameter.as_str()))
            .collect();
        assert_eq!(clauses, expected);
    }
    let head = library.head().await?;
    let namespace = library.record_store().namespace();
    let reopened = fixtures::open_library(db.clone(), actor, Some(namespace)).await?;
    reopened.import_basic().await?;
    assert_eq!(reopened.head().await?, head);
    Ok(())
}
#[tokio::test]
async fn authored_batch_failure_keeps_prior_commits_and_restart_skips_them() -> Result<()> {
    let local = SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    seed(&local, false).await?;
    local.db.execute_unprepared("CREATE TRIGGER reject_batch BEFORE INSERT ON sync_journal WHEN NEW.sequence=130 BEGIN SELECT RAISE(ABORT, 'injected batch failure'); END").await?;
    let error = format!("{:#}", local.sync_store.import_basic().await.unwrap_err());
    assert!(error.contains("injected batch failure"), "{error}");
    assert_eq!(local.sync_store.head().await?.sequence, 126);
    local
        .db
        .execute_unprepared("DROP TRIGGER reject_batch")
        .await?;
    finish_and_check(&local).await
}
