use anyhow::Result;
use database::test_fixtures::causal_library as support;
use database::{
    actions::stats::increase_played_through, entities::media_file_stats, sync::model::*,
};
use sea_orm::{EntityTrait, TransactionTrait};
use support::{SqlLibrary, exchange, key};
use sync::{self as p, SyncModel, SyncStore};
use uuid::Uuid;

// A peer can construct bytes without enforcing Rune's ownership rule.
// The receiving Rune endpoint must independently reject such a value.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Unchecked;
impl SyncModel for Unchecked {
    type Collection = Collection;
    type Payload = Payload;
    fn model_id() -> &'static str {
        RuneModel::model_id()
    }
    fn schema_id() -> &'static str {
        RuneModel::schema_id()
    }
    fn collections() -> Vec<Collection> {
        RuneModel::collections()
    }
    fn collection_tag(value: &Collection) -> &'static str {
        RuneModel::collection_tag(value)
    }
    fn collection_from_tag(tag: &str) -> Option<Collection> {
        RuneModel::collection_from_tag(tag)
    }
    fn validate(_: &p::ObjectKey<Self>, _: &Option<Payload>) -> p::Result<()> {
        Ok(())
    }
}
#[tokio::test]
async fn statistics_owner_is_checked_and_relays_preserve_the_author() -> Result<()> {
    let owner = Uuid::from_u128(1);
    let relay = Uuid::from_u128(2);
    let namespace = Uuid::new_v4();
    let track = Uuid::new_v4();
    let a = SqlLibrary::new(namespace, owner).await?;
    let b = SqlLibrary::new(namespace, relay).await?;
    let c = SqlLibrary::new(namespace, Uuid::from_u128(3)).await?;
    for local in [&a, &b, &c] {
        RuneModel::project(
            &local.db,
            &key(Collection::Tracks, track),
            Some(&support::track()),
        )
        .await?;
    }
    support::fixture_statistics(&b.db, 1, owner, 2, 3, "original").await?;
    let before = media_file_stats::Entity::find().one(&b.db).await?.unwrap();
    let head = b.peer.head().await?;
    assert!(
        increase_played_through(&b.db, &b.sync_store.record_store(), 1, owner)
            .await
            .is_err()
    );
    assert_eq!(
        media_file_stats::Entity::find().one(&b.db).await?.unwrap(),
        before
    );
    assert_eq!(b.peer.head().await?, head);
    let object = key(
        Collection::PlaybackStatistics,
        Uuid::new_v5(&track, owner.as_bytes()),
    );
    let identity = p::StoreIdentity {
        namespace,
        model: p::ModelId {
            model: RuneModel::model_id().into(),
            schema: RuneModel::schema_id().into(),
        },
        replica: Uuid::new_v4(),
        generation: Uuid::new_v4(),
    };
    let dishonest = p::MemorySyncStore::<Unchecked>::new(
        identity.clone(),
        p::LocalAuthor {
            author: relay,
            writer_incarnation: Uuid::new_v4(),
        },
    )?;
    let after = dishonest.head().await?;
    let scope = p::Scope::<Unchecked> {
        collections: [Collection::PlaybackStatistics].into_iter().collect(),
    };
    let checkpoint = p::CheckpointKey {
        namespace,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: p::scope_id(&scope)?,
    };
    let b_identity = b.sync_store.identity().await?;
    let tx = b.db.begin().await?;
    sync_orm::causal_progress::initialize_checkpoint_on(
        &tx,
        &b_identity,
        &checkpoint,
        &Scope {
            collections: scope.collections.clone(),
        },
        &after,
    )
    .await?;
    tx.commit().await?;
    dishonest
        .write_local(
            p::ObjectKey {
                collection: object.collection,
                id: object.id,
            },
            Some(Payload::PlaybackStatistics(PlaybackStatistics {
                track_id: track,
                device_id: owner,
                skipped: 8,
                played_through: 9,
                updated_at: "forged".into(),
            })),
        )
        .await?;
    let page = dishonest
        .read_page(p::PageRequest {
            source: identity,
            scope,
            after: after.clone(),
            through: dishonest.head().await?,
            limits: p::PageLimits {
                max_scanned: 4,
                max_bytes: 65536,
            },
        })
        .await?;
    let p::PageRead::Page(page): p::PageRead<RuneModel> =
        serde_json::from_slice(&serde_json::to_vec(&page)?)?
    else {
        anyhow::bail!("fixture exceeded one page");
    };
    let error = b
        .peer
        .apply_page(&page, database::sync::session_limits().apply)
        .await
        .unwrap_err();
    assert!(
        format!("{error:?}").contains("unauthorized author"),
        "{error}"
    );
    assert_eq!(
        media_file_stats::Entity::find().one(&b.db).await?.unwrap(),
        before
    );
    assert_eq!(b.peer.head().await?, head);
    assert!(b.peer.get(&object).await?.is_none());
    assert_eq!(b.peer.checkpoint(&checkpoint).await?, Some(after));
    increase_played_through(&a.db, &a.sync_store.record_store(), 1, owner).await?;
    exchange(&a.peer, &b.peer).await?;
    assert_eq!(b.peer.get(&object).await?.unwrap().revision.actor, owner);
    exchange(&b.peer, &c.peer).await?;
    assert_eq!(c.peer.get(&object).await?.unwrap().revision.actor, owner);
    let received = media_file_stats::Entity::find().one(&c.db).await?.unwrap();
    assert_eq!(received.device_id, owner.to_string());
    assert_eq!(received.played_through, 1);
    Ok(())
}
