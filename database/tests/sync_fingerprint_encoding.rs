use database::test_fixtures::causal_library as support;

use anyhow::Result;
use async_trait::async_trait;
use database::{
    actions::stats::increase_played_through,
    entities::{media_file_stats, media_files},
    sync::{
        LibrarySyncStore,
        model::{Collection, Fingerprint, ObjectKey, Payload, PlaybackStatistics, RuneModel},
    },
};
use migration::{Migrator, MigratorTrait, SchemaManager, catalog_identity};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, EntityTrait,
    QueryFilter,
    sea_query::{Alias, Expr, ExprTrait, Query, Table},
};
use std::sync::Arc;
use sync::protocol::{
    self as p, BoxFuture, Candidate, CheckpointKey, Dot, HistoryStatus, ModelId, PageRead,
    PageRequest, ReceiveProgress, Revision, StoreIdentity, SyncModel, SyncPeer, SyncStore,
};
use sync_orm::{
    causal_capabilities::{CausalEndpoint, TransactionProjection},
    causal_store::{CausalReplica, namespace_model_on},
    row_policy::BinaryValue,
};
use uuid::Uuid;

fn fingerprint_payload(bytes: Vec<u8>) -> Payload {
    Payload::Fingerprint(Fingerprint {
        file_id: Uuid::from_u128(1),
        bytes: BinaryValue(bytes),
        is_duplicated: 0,
        source_hash: None,
        source_size: None,
        algorithm: None,
        configuration: None,
        format_version: None,
    })
}

fn candidate(payload: Payload) -> Candidate<RuneModel> {
    Candidate {
        model: ModelId {
            model: RuneModel::model_id().into(),
            schema: RuneModel::schema_id().into(),
        },
        namespace: Uuid::from_u128(10),
        key: ObjectKey {
            collection: Collection::Fingerprints,
            id: catalog_identity::fingerprint(Uuid::from_u128(1), None, None, None),
        },
        dot: Dot {
            writer_incarnation: Uuid::from_u128(3),
            counter: 1,
        },
        revision: Revision {
            actor: Uuid::from_u128(4),
            counter: 1,
        },
        payload,
    }
}

#[test]
fn fingerprint_canonical_json_uses_stable_base64_bytes() -> Result<()> {
    let actual = String::from_utf8(p::canonical_candidate(&candidate(fingerprint_payload(
        vec![0xfb, 0xff],
    )))?)?;
    let expected = r#"{"dot":{"counter":1,"writer_incarnation":"00000000-0000-0000-0000-000000000003"},"key":{"collection":"fingerprints","id":"8bd90792-2ce2-5180-b663-edee773c25c1"},"model":{"model":"rune.library","schema":"2"},"namespace":"00000000-0000-0000-0000-00000000000a","payload":{"collection":"Fingerprint","value":{"algorithm":null,"bytes":"+/8=","configuration":null,"file_id":"00000000-0000-0000-0000-000000000001","format_version":null,"is_duplicated":0,"source_hash":null,"source_size":null}},"revision":{"actor":"00000000-0000-0000-0000-000000000004","counter":1}}"#;
    assert_eq!(actual, expected);
    let decoded = p::decode_candidate::<RuneModel>(actual.as_bytes())?;
    assert_eq!(decoded.payload, fingerprint_payload(vec![0xfb, 0xff]));
    Ok(())
}

#[test]
fn maximum_fingerprint_round_trips_in_a_bounded_candidate() -> Result<()> {
    let bytes: Vec<u8> = (0..32768).map(|index| (index % 251) as u8).collect();
    let candidate = candidate(fingerprint_payload(bytes.clone()));
    let encoded = p::canonical_candidate(&candidate)?;
    assert!(encoded.len() as u64 <= p::MAX_CANDIDATE_BYTES);
    let decoded = p::decode_candidate::<RuneModel>(&encoded)?;
    let Payload::Fingerprint(decoded) = decoded.payload else {
        anyhow::bail!("decoded wrong payload family")
    };
    assert_eq!(decoded.bytes.0, bytes);
    Ok(())
}

#[tokio::test]
async fn two_sql_stores_sync_a_maximum_fingerprint() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let file_id = Uuid::from_u128(1);
    let fingerprint_id = catalog_identity::fingerprint(file_id, None, None, None);
    let value = match fingerprint_payload(vec![0x5a; 32768]) {
        Payload::Fingerprint(value) => value,
        _ => unreachable!(),
    };
    source
        .peer
        .write_local(
            support::key(Collection::Fingerprints, fingerprint_id),
            Some(Payload::Fingerprint(value.clone())),
        )
        .await?;
    support::exchange(&source.peer, &target.peer).await?;
    assert_eq!(
        target
            .peer
            .get(&support::key(Collection::Fingerprints, fingerprint_id))
            .await?
            .and_then(|record| record.value),
        Some(Payload::Fingerprint(value))
    );
    Ok(())
}

#[tokio::test]
async fn old_local_encoding_is_rebuilt_from_binary_source_rows() -> Result<()> {
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let local = support::SqlLibrary::new(namespace, actor).await?;
    let fingerprint_id = catalog_identity::fingerprint(Uuid::from_u128(1), None, None, None);
    let value = match fingerprint_payload(vec![0xa5; 128]) {
        Payload::Fingerprint(value) => value,
        _ => unreachable!(),
    };
    local
        .peer
        .write_local(
            support::key(Collection::Fingerprints, fingerprint_id),
            Some(Payload::Fingerprint(value.clone())),
        )
        .await?;
    let mut update = Query::update()
        .table(Alias::new("sync_meta"))
        .value(Alias::new("schema"), "1")
        .to_owned();
    update.and_where(
        Expr::col(Alias::new("namespace")).eq(Expr::value(namespace.as_bytes().to_vec())),
    );
    local.db.execute(&update).await?;

    let reopened = LibrarySyncStore::open(local.db.clone(), actor, Some(namespace)).await?;
    assert_eq!(
        reopened
            .get(&support::key(Collection::Fingerprints, fingerprint_id))
            .await?
            .and_then(|record| record.value),
        Some(Payload::Fingerprint(value))
    );
    assert_eq!(
        namespace_model_on(&local.db, namespace)
            .await?
            .map(|model| model.schema),
        Some(RuneModel::schema_id().to_owned())
    );
    Ok(())
}

#[tokio::test]
async fn old_schema_peer_is_rejected_with_typed_incompatibility() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let old_peer = OldSchemaPeer(source.peer.clone());
    let target_endpoint = target.sync_store.endpoint();
    let source_endpoint = source.sync_store.endpoint();
    let mut coordinator = p::Coordinator::<RuneModel>::new(support::scope(), support::limits())?;
    assert!(matches!(
        coordinator
            .advance(
                &target_endpoint,
                &target_endpoint,
                &target_endpoint,
                &old_peer,
                &source_endpoint,
            )
            .await?,
        p::SessionAdvance::Incomplete
    ));
    let error = coordinator
        .advance(
            &target_endpoint,
            &target_endpoint,
            &target_endpoint,
            &old_peer,
            &source_endpoint,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, p::SyncError::IncompatibleModel));
    Ok(())
}

struct OldSchemaPeer(support::Peer);

impl SyncPeer<RuneModel> for OldSchemaPeer {
    fn identity(&self) -> BoxFuture<'_, StoreIdentity> {
        Box::pin(async move {
            let mut identity = self.0.identity().await?;
            identity.model.schema = "1".into();
            Ok(identity)
        })
    }
    fn head(&self) -> BoxFuture<'_, ReceiveProgress> {
        self.0.head()
    }
    fn history_status<'a>(
        &'a self,
        key: &'a CheckpointKey,
        progress: Option<ReceiveProgress>,
    ) -> BoxFuture<'a, HistoryStatus> {
        self.0.history_status(key, progress)
    }
    fn read_page(&self, request: PageRequest<RuneModel>) -> BoxFuture<'_, PageRead<RuneModel>> {
        self.0.read_page(request)
    }
    fn read_fragment(
        &self,
        manifest: p::ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, p::Fragment> {
        p::SyncStore::read_fragment(&*self.0, manifest, ordinal, max_bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Schema1Model;

impl SyncModel for Schema1Model {
    type Collection = Collection;
    type Payload = Payload;

    fn model_id() -> &'static str {
        RuneModel::model_id()
    }

    fn schema_id() -> &'static str {
        "1"
    }

    fn collections() -> Vec<Collection> {
        RuneModel::collections()
    }

    fn collection_tag(collection: &Collection) -> &'static str {
        RuneModel::collection_tag(collection)
    }

    fn collection_from_tag(tag: &str) -> Option<Collection> {
        RuneModel::collection_from_tag(tag)
    }

    fn validate(key: &p::ObjectKey<Self>, value: &Option<Payload>) -> p::Result<()> {
        RuneModel::validate(
            &ObjectKey {
                collection: key.collection,
                id: key.id,
            },
            value,
        )
    }

    fn validate_revision(
        key: &p::ObjectKey<Self>,
        value: &Option<Payload>,
        revision: &Revision,
    ) -> p::Result<()> {
        RuneModel::validate_revision(
            &ObjectKey {
                collection: key.collection,
                id: key.id,
            },
            value,
            revision,
        )
    }
}

struct Schema1Projection;

#[async_trait]
impl TransactionProjection<Schema1Model> for Schema1Projection {
    fn mutation_ceiling(&self, changes: &p::ProjectionBatch<Schema1Model>) -> Result<u64> {
        Ok((changes.changes.len() as u64).saturating_mul(16))
    }

    async fn apply(
        &self,
        tx: &DatabaseTransaction,
        changes: &p::ProjectionBatch<Schema1Model>,
    ) -> Result<()> {
        for change in &changes.changes {
            let key = ObjectKey {
                collection: change.key.collection,
                id: change.key.id,
            };
            RuneModel::project_on(
                tx,
                &key,
                change.after.as_ref().map(|candidate| &candidate.payload),
            )
            .await?;
        }
        Ok(())
    }
}

struct Schema1Library {
    db: DatabaseConnection,
    endpoint: CausalEndpoint<Schema1Model>,
}

impl Schema1Library {
    async fn new(namespace: Uuid, actor: Uuid) -> Result<Self> {
        let db = support::fixtures::memory_database(actor).await?;
        let replica =
            CausalReplica::<Schema1Model>::create_empty(db.clone(), namespace, actor).await?;
        let endpoint = CausalEndpoint::with_projection(replica, Arc::new(Schema1Projection));
        Ok(Self { db, endpoint })
    }

    async fn exchange_to(&self, target: &Self) -> Result<()> {
        let scope = p::Scope::<Schema1Model> {
            collections: Schema1Model::collections().into_iter().collect(),
        };
        let remote = p::StorePeer(Arc::new(self.endpoint.clone()));
        let mut coordinator = p::Coordinator::new(scope, support::limits())?;
        for _ in 0..20000 {
            match coordinator
                .advance(
                    &target.endpoint,
                    &target.endpoint,
                    &target.endpoint,
                    &remote,
                    &self.endpoint,
                )
                .await?
            {
                p::SessionAdvance::Complete(_) => return Ok(()),
                p::SessionAdvance::BudgetExhausted => coordinator.start_pass(),
                p::SessionAdvance::Incomplete => (),
                p::SessionAdvance::Cancelled => anyhow::bail!("unexpected cancellation"),
            }
        }
        anyhow::bail!("schema 1 exchange did not complete")
    }
}

fn schema1_key(collection: Collection, id: Uuid) -> p::ObjectKey<Schema1Model> {
    p::ObjectKey { collection, id }
}

fn statistics_key(track_id: Uuid, device_id: Uuid) -> ObjectKey {
    support::key(
        Collection::PlaybackStatistics,
        Uuid::new_v5(&track_id, device_id.as_bytes()),
    )
}

#[tokio::test]
async fn schema_1_upgrade_preserves_remote_accepted_statistics_and_forwards_them_to_new_device()
-> Result<()> {
    let namespace = Uuid::new_v4();
    let actor_a = Uuid::new_v4();
    let actor_b = Uuid::new_v4();
    let actor_c = Uuid::new_v4();
    let actor_unaccepted = Uuid::new_v4();
    let track_id = Uuid::new_v4();

    let a = Schema1Library::new(namespace, actor_a).await?;
    let b = Schema1Library::new(namespace, actor_b).await?;

    a.endpoint
        .write_local(
            schema1_key(Collection::Tracks, track_id),
            Some(support::track()),
        )
        .await?;
    let a_stats = PlaybackStatistics {
        track_id,
        device_id: actor_a,
        skipped: 1,
        played_through: 2,
        updated_at: "2026-11-01T00:00:00Z".into(),
    };
    let a_stats_key = statistics_key(track_id, actor_a);
    let a_written = a
        .endpoint
        .write_local(
            schema1_key(Collection::PlaybackStatistics, a_stats_key.id),
            Some(Payload::PlaybackStatistics(a_stats.clone())),
        )
        .await?;

    a.exchange_to(&b).await?;

    let b_stats = PlaybackStatistics {
        track_id,
        device_id: actor_b,
        skipped: 3,
        played_through: 7,
        updated_at: "2026-11-02T00:00:00Z".into(),
    };
    let b_stats_key = statistics_key(track_id, actor_b);
    b.endpoint
        .write_local(
            schema1_key(Collection::PlaybackStatistics, b_stats_key.id),
            Some(Payload::PlaybackStatistics(b_stats.clone())),
        )
        .await?;
    b.exchange_to(&a).await?;

    // Unaccepted foreign SQL row must remain unauthored across the upgrade.
    let track_local_id = media_files::Entity::find()
        .filter(media_files::Column::SourceUuid.eq(track_id.to_string()))
        .one(&a.db)
        .await?
        .unwrap()
        .id;
    support::fixture_statistics(&a.db, track_local_id, actor_unaccepted, 9, 11, "unaccepted")
        .await?;
    let unaccepted_key = statistics_key(track_id, actor_unaccepted);

    assert_eq!(
        namespace_model_on(&a.db, namespace)
            .await?
            .map(|model| model.schema),
        Some("1".to_owned())
    );
    assert_eq!(
        a.endpoint
            .get(&schema1_key(Collection::PlaybackStatistics, b_stats_key.id))
            .await?
            .unwrap()
            .revision
            .actor,
        actor_b
    );

    // Device B is unavailable while A upgrades and syncs to C.
    drop(b);

    // The fixture projects into the current catalog. Roll the schema back after seeding it.
    let migrations = Migrator::migrations();
    let prior = migrations
        .iter()
        .position(|m| m.name() == "m20261124_000040_create_sync_export_cursors")
        .expect("export cursor migration must be registered");
    Migrator::down(&a.db, Some(u32::try_from(migrations.len() - prior)?)).await?;

    // Model a library that synced statistics before export cursors existed.
    a.db.execute(
        &Table::drop()
            .table(Alias::new("sync_export_cursors"))
            .to_owned(),
    )
    .await?;
    Migrator::up(&a.db, None).await?;
    assert!(
        SchemaManager::new(&a.db)
            .has_table("sync_export_cursors")
            .await?,
        "the Migrator creates export cursors before the legacy namespace is opened"
    );

    let upgraded_a = LibrarySyncStore::open(a.db.clone(), actor_a, Some(namespace)).await?;
    assert_eq!(
        namespace_model_on(&a.db, namespace)
            .await?
            .map(|model| model.schema),
        Some(RuneModel::schema_id().to_owned())
    );

    // B's SQL row and accepted sync record are both present on A with B's ownership preserved.
    let a_sql_b = media_file_stats::Entity::find()
        .filter(media_file_stats::Column::DeviceId.eq(actor_b.to_string()))
        .one(&a.db)
        .await?
        .expect("B's SQL row should remain on A after upgrade");
    assert_eq!(a_sql_b.skipped, 3);
    assert_eq!(a_sql_b.played_through, 7);
    assert_eq!(a_sql_b.updated_at, "2026-11-02T00:00:00Z");

    let accepted_b = upgraded_a
        .get(&b_stats_key)
        .await?
        .expect("B's accepted statistics record must survive schema 1 -> 2 upgrade");
    assert_eq!(accepted_b.revision.actor, actor_b);
    assert_eq!(
        accepted_b.value,
        Some(Payload::PlaybackStatistics(b_stats.clone()))
    );

    let accepted_a = upgraded_a
        .get(&a_stats_key)
        .await?
        .expect("A's local statistics record must survive schema 1 -> 2 upgrade");
    assert_eq!(accepted_a.revision.actor, actor_a);
    assert_eq!(
        accepted_a.value,
        Some(Payload::PlaybackStatistics(a_stats.clone()))
    );

    assert!(
        upgraded_a.get(&unaccepted_key).await?.is_none(),
        "unaccepted foreign SQL statistics row must not be authored during upgrade"
    );

    // Repeat imports and reopen remain idempotent.
    let head_after_upgrade = upgraded_a.head().await?;
    upgraded_a.import_existing(actor_a).await?;
    assert_eq!(upgraded_a.head().await?, head_after_upgrade);
    let reopened_a = LibrarySyncStore::open(a.db.clone(), actor_a, Some(namespace)).await?;
    reopened_a.import_existing(actor_a).await?;
    assert_eq!(reopened_a.head().await?, head_after_upgrade);

    // Sync upgraded A to a brand-new Schema 2 device C while B is unavailable.
    let c = support::SqlLibrary::new(namespace, actor_c).await?;
    let a_peer: support::Peer = Arc::new(reopened_a.endpoint());
    support::exchange(&a_peer, &c.peer).await?;

    let c_accepted_b = c
        .peer
        .get(&b_stats_key)
        .await?
        .expect("C must receive B's accepted statistics from upgraded A");
    assert_eq!(c_accepted_b.revision.actor, actor_b);
    assert_eq!(
        c_accepted_b.value,
        Some(Payload::PlaybackStatistics(b_stats))
    );

    let c_rows = media_file_stats::Entity::find().all(&c.db).await?;
    assert_eq!(c_rows.len(), 2);
    let c_row_b = c_rows
        .iter()
        .find(|row| row.device_id == actor_b.to_string())
        .expect("C must project B's SQL statistics row");
    assert_eq!(c_row_b.track_uuid, track_id.to_string());
    assert_eq!(c_row_b.skipped, 3);
    assert_eq!(c_row_b.played_through, 7);
    assert_eq!(c_row_b.updated_at, "2026-11-02T00:00:00Z");

    let c_row_a = c_rows
        .iter()
        .find(|row| row.device_id == actor_a.to_string())
        .expect("C must project A's SQL statistics row");
    assert_eq!(c_row_a.skipped, 1);
    assert_eq!(c_row_a.played_through, 2);

    // Local statistics mutations on A continue working and advance monotonically.
    increase_played_through(&a.db, &reopened_a.record_store(), track_local_id, actor_a).await?;
    let updated_a = reopened_a.get(&a_stats_key).await?.unwrap();
    assert_eq!(updated_a.revision.actor, actor_a);
    assert!(updated_a.revision.counter > a_written.revision.counter);
    support::exchange(&a_peer, &c.peer).await?;
    let c_updated_a = media_file_stats::Entity::find()
        .filter(media_file_stats::Column::DeviceId.eq(actor_a.to_string()))
        .one(&c.db)
        .await?
        .unwrap();
    assert_eq!(c_updated_a.played_through, 3);

    Ok(())
}

#[tokio::test]
async fn schema_1_upgrade_batches_many_remote_statistics_and_preserves_owner_updates() -> Result<()>
{
    let namespace = Uuid::new_v4();
    let actor_a = Uuid::new_v4();
    let actor_b = Uuid::new_v4();
    let actor_c = Uuid::new_v4();

    let a = Schema1Library::new(namespace, actor_a).await?;
    let b = Schema1Library::new(namespace, actor_b).await?;

    let mut tracks = Vec::with_capacity(70);
    for index in 0..70_i32 {
        let track_id = Uuid::new_v4();
        tracks.push(track_id);
        b.endpoint
            .write_local(
                schema1_key(Collection::Tracks, track_id),
                Some(support::track()),
            )
            .await?;
        let stats_key = statistics_key(track_id, actor_b);
        b.endpoint
            .write_local(
                schema1_key(Collection::PlaybackStatistics, stats_key.id),
                Some(Payload::PlaybackStatistics(PlaybackStatistics {
                    track_id,
                    device_id: actor_b,
                    skipped: index,
                    played_through: index + 1,
                    updated_at: format!("2026-11-02T00:00:{:02}Z", index % 60),
                })),
            )
            .await?;
    }
    b.exchange_to(&a).await?;

    let upgraded_a = LibrarySyncStore::open(a.db.clone(), actor_a, Some(namespace)).await?;
    let c = support::SqlLibrary::new(namespace, actor_c).await?;
    let a_peer: support::Peer = Arc::new(upgraded_a.endpoint());
    support::exchange(&a_peer, &c.peer).await?;
    assert_eq!(media_file_stats::Entity::find().all(&c.db).await?.len(), 70);

    // When B also upgrades and increments its own statistics, B's update beats the restored
    // Schema 1 record rather than being overwritten by A's pre-upgrade copy.
    let upgraded_b = LibrarySyncStore::open(b.db.clone(), actor_b, Some(namespace)).await?;
    let first_track = tracks[0];
    let b_first_local_id = media_files::Entity::find()
        .filter(media_files::Column::SourceUuid.eq(first_track.to_string()))
        .one(&b.db)
        .await?
        .unwrap()
        .id;
    increase_played_through(&b.db, &upgraded_b.record_store(), b_first_local_id, actor_b).await?;
    let b_peer: support::Peer = Arc::new(upgraded_b.endpoint());
    support::exchange(&b_peer, &a_peer).await?;
    support::exchange(&a_peer, &b_peer).await?;

    let a_first_b = media_file_stats::Entity::find()
        .filter(media_file_stats::Column::TrackUuid.eq(first_track.to_string()))
        .filter(media_file_stats::Column::DeviceId.eq(actor_b.to_string()))
        .one(&a.db)
        .await?
        .unwrap();
    assert_eq!(a_first_b.played_through, 2);

    Ok(())
}
