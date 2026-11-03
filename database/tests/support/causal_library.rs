#![allow(dead_code)]

use std::time::Instant;
#[path = "fixtures.rs"]
pub mod fixtures;

use anyhow::Result;
use database::entities::media_files;
use database::sync::{LibrarySyncStore, model::*};
use http_request::sync::MetadataStore;
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection, TransactionTrait};
use std::sync::Arc;
use sync::{self as p, SyncModel};
use uuid::Uuid;

pub type Peer = Arc<dyn MetadataStore<RuneModel>>;

pub fn memory(namespace: Uuid, actor: Uuid) -> Result<Peer> {
    Ok(Arc::new(MemorySyncStore::new(
        p::StoreIdentity {
            namespace,
            model: p::ModelId {
                model: RuneModel::model_id().into(),
                schema: RuneModel::schema_id().into(),
            },
            replica: actor,
            generation: Uuid::new_v4(),
        },
        p::LocalAuthor {
            author: actor,
            writer_incarnation: Uuid::new_v4(),
        },
    )?))
}

pub struct SqlLibrary {
    pub db: DatabaseConnection,
    pub library: LibrarySyncStore,
    pub peer: Peer,
    pub root: tempfile::TempDir,
}
impl SqlLibrary {
    pub async fn initialize_from(
        &self,
        source: &Peer,
        scope: &Scope,
    ) -> Result<p::ReceiveProgress> {
        let identity = source.identity().await?;
        let head = source.head().await?;
        let checkpoint = p::CheckpointKey {
            namespace: identity.namespace,
            source_replica: identity.replica,
            source_generation: identity.generation,
            scope_id: p::scope_id(scope)?,
        };
        let local = self.library.identity().await?;
        let tx = self.db.begin().await?;
        sync_orm::causal_progress::initialize_checkpoint_on(&tx, &local, &checkpoint, scope, &head)
            .await?;
        tx.commit().await?;
        Ok(head)
    }
    pub async fn new(namespace: Uuid, actor: Uuid) -> Result<Self> {
        let _ = migration::initialize_node_id(actor.to_string());
        let root = tempfile::tempdir()?;
        let options = fixtures::single_connection_options();
        let db = Database::connect(options).await?;
        Migrator::up(&db, None).await?;
        let library = fixtures::open_library(db.clone(), actor, Some(namespace)).await?;
        let peer = Arc::new(library.endpoint());
        Ok(Self {
            db,
            library,
            peer,
            root,
        })
    }
}

pub fn scope() -> Scope {
    Scope {
        collections: RuneModel::collections().into_iter().collect(),
    }
}

pub fn limits() -> p::SessionLimits {
    let mut limits = database::sync::session_limits();
    limits.pages.max_scanned = 3;
    limits.pages.max_bytes = 65536;
    limits.apply.max_projection_items = 3;
    limits.work.max_requests = 16;
    limits.work.max_rows_scanned = 128;
    limits
}

pub async fn exchange(source: &Peer, target: &Peer) -> Result<usize> {
    exchange_scoped(source, target, scope()).await
}

pub async fn exchange_scoped(source: &Peer, target: &Peer, scope: Scope) -> Result<usize> {
    let remote = p::StorePeer(source.clone());
    drive(&**target, &**target, &**target, &remote, &**source, scope).await
}

pub async fn drive(
    receiver: &dyn p::ReceiveStore<RuneModel>,
    sink: &dyn p::SnapshotSink<RuneModel>,
    local_source: &dyn p::SnapshotSource<RuneModel>,
    peer: &dyn p::SyncPeer<RuneModel>,
    remote_source: &dyn p::SnapshotSource<RuneModel>,
    scope: Scope,
) -> Result<usize> {
    let started = Instant::now();
    let mut coordinator = p::Coordinator::new(scope, limits())?;
    for step in 0..20000 {
        match coordinator
            .advance(receiver, sink, local_source, peer, remote_source)
            .await?
        {
            p::SessionAdvance::Complete(progress) => {
                if std::env::var_os("RUNE_SYNC_TRACE").is_some() {
                    eprintln!(
                        "exchange steps={step} elapsed_ms={} through={}",
                        started.elapsed().as_millis(),
                        progress.sequence
                    );
                }
                return Ok(step);
            }
            p::SessionAdvance::BudgetExhausted => coordinator.start_pass(),
            p::SessionAdvance::Incomplete => (),
            p::SessionAdvance::Cancelled => anyhow::bail!("unexpected cancellation"),
        }
    }
    anyhow::bail!("bounded session did not complete")
}

pub fn key(collection: Collection, id: Uuid) -> ObjectKey {
    ObjectKey { collection, id }
}

pub fn playlist(name: &str) -> Option<Payload> {
    Some(Payload::Playlist(Playlist {
        name: name.into(),
        group: "test".into(),
    }))
}

pub fn track() -> Payload {
    Payload::Track(Track {
        extension: "flac".into(),
        content_hash: Some("a".repeat(64)),
        content_size: Some(100),
        duration: "1.23456789".to_owned().try_into().unwrap(),
        sample_rate: 48000,
        cover_art_id: None,
        metadata: vec![MetadataTag {
            id: Uuid::from_u128(90001),
            position: 0,
            key: "track_title".into(),
            value: "Referenced track".into(),
        }],
    })
}

pub fn audio(title: impl Into<String>, blob: &BlobRef) -> Payload {
    Payload::Track(Track {
        extension: "ogg".into(),
        content_hash: Some(blob.digest.clone()),
        content_size: Some(blob.size.try_into().unwrap()),
        duration: "0".to_owned().try_into().unwrap(),
        sample_rate: 0,
        cover_art_id: None,
        metadata: vec![MetadataTag {
            id: Uuid::new_v4(),
            position: 0,
            key: "track_title".into(),
            value: title.into(),
        }],
    })
}

pub async fn local_file(
    db: &DatabaseConnection,
    id: i32,
    uuid: Uuid,
    name: &str,
) -> Result<media_files::Model> {
    use database::entities::media_files;
    use sea_orm::{EntityTrait, Set};
    Ok(media_files::Entity::insert(media_files::ActiveModel {
        id: Set(id),
        hlc_uuid: Set(uuid.to_string()),
        file_name: Set(name.into()),
        directory: Set(String::new()),
        extension: Set("ogg".into()),
        file_hash: Set(name.into()),
        last_modified: Set("old".into()),
        sample_rate: Set(44100),
        duration: Set(1.into()),
        local_available: Set(true),
        ..Default::default()
    })
    .exec_with_returning(db)
    .await?)
}

/// Project a statistics row as a test fixture without authoring a local command.
pub async fn fixture_statistics(
    db: &DatabaseConnection,
    file_id: i32,
    device: Uuid,
    skipped: i32,
    played: i32,
    updated_at: &str,
) -> Result<()> {
    use sea_orm::EntityTrait;
    let file = media_files::Entity::find_by_id(file_id)
        .one(db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("fixture track missing"))?;
    let track: Uuid = file.hlc_uuid.parse()?;
    let key = key(
        Collection::PlaybackStatistics,
        Uuid::new_v5(&track, device.as_bytes()),
    );
    let payload = Payload::PlaybackStatistics(PlaybackStatistics {
        track_id: track,
        device_id: device,
        skipped,
        played_through: played,
        updated_at: updated_at.into(),
    });
    RuneModel::project(db, &key, Some(&payload)).await?;
    Ok(())
}

/// Project a like row as a test fixture without authoring a local command.
pub async fn fixture_like(
    db: &DatabaseConnection,
    file_id: i32,
    liked: bool,
    updated_at: &str,
) -> Result<()> {
    use sea_orm::EntityTrait;
    let file = media_files::Entity::find_by_id(file_id)
        .one(db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("fixture track missing"))?;
    let track: Uuid = file.hlc_uuid.parse()?;
    let payload = Payload::Liked(Liked {
        track_id: track,
        liked,
        updated_at: updated_at.into(),
    });
    RuneModel::project(db, &key(Collection::Likes, track), Some(&payload)).await?;
    Ok(())
}

pub fn metadata_track(tags: Vec<MetadataTag>) -> Payload {
    Payload::Track(Track {
        extension: "flac".into(),
        content_hash: None,
        content_size: None,
        duration: "1.23456789".to_owned().try_into().unwrap(),
        sample_rate: 48000,
        cover_art_id: None,
        metadata: tags,
    })
}
