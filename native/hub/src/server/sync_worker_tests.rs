use super::*;
use database::credentials::{Credential, CredentialRef, CredentialStore, Secret};
use database::library_registry::{CatalogPlacement, FsAdapter, LibraryRecordId, ProtocolAdapter};
use database::sync::LibrarySyncStore;
use database::sync::model::{Collection, ObjectKey, RuneModel};
use std::sync::Arc;
use sync::SyncModel;
use sync::share::{
    FORMAT_VERSION, LibraryFile, MemoryObjectStore, MemoryStoreConfig, ObjectStore, StoreError,
    StoreOperation, encode_library, library_key,
};
use uuid::Uuid;

#[path = "share_worker_tests/support.rs"]
mod support;

struct MemoryShareFactory {
    store: Arc<MemoryObjectStore>,
}

impl ShareStoreFactory for MemoryShareFactory {
    fn open<'a>(
        &'a self,
        _record: &'a LibraryRecord,
        _prefix: &'a str,
        _credentials: &'a CredentialStore,
    ) -> BoxFuture<'a, Result<ShareStoreSession, ShareSyncFailure>> {
        let store: Arc<dyn ObjectStore> = self.store.clone();
        Box::pin(async move {
            Ok(ShareStoreSession {
                store,
                capabilities: ShareCapabilities::default(),
            })
        })
    }
}

fn test_record(library_id: Uuid) -> LibraryRecord {
    LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: "share-test".into(),
        library_id: Some(library_id),
        protocol: ProtocolAdapter::Native,
        fs: vec![FsAdapter::WebDav {
            endpoint: "http://localhost".into(),
            root: "/share".into(),
            username: "user".into(),
            credential: CredentialRef(Uuid::new_v4()),
        }],
        catalog: CatalogPlacement::InShare(Uuid::new_v4()),
        background_sync: true,
    }
}

async fn memory_session(store: &Arc<MemoryObjectStore>) -> ShareStoreSession {
    let factory = MemoryShareFactory {
        store: store.clone(),
    };
    let directory = tempfile::tempdir().unwrap();
    let credentials = CredentialStore::open(directory.path());
    factory
        .open(&test_record(Uuid::new_v4()), "probe-prefix", &credentials)
        .await
        .unwrap()
}

async fn sync_pass(
    library: &LibrarySyncStore,
    library_id: Uuid,
    session: &ShareStoreSession,
) -> Result<(), ShareSyncFailure> {
    let cache = std::env::temp_dir().join(format!("hub-share-{}", Uuid::new_v4()));
    share_sync_pass(
        library,
        library_id,
        cache,
        session,
        SharePass::PullAndPush,
        || {},
    )
    .await
    .map(|_| ())
}

#[tokio::test]
async fn worker_factory_share_passes_converge_and_relay_edits() -> Result<()> {
    let namespace = Uuid::new_v4();
    let first = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let second = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    let session = memory_session(&store).await;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    first
        .library
        .write_local(key.clone(), support::playlist("from first device"))
        .await?;
    sync_pass(&first.library, namespace, &session)
        .await
        .unwrap();
    sync_pass(&second.library, namespace, &session)
        .await
        .unwrap();
    assert!(second.library.get(&key).await?.is_some());

    let next_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    first
        .library
        .write_local(next_key.clone(), support::playlist("later edit"))
        .await?;
    sync_pass(&first.library, namespace, &session)
        .await
        .unwrap();
    sync_pass(&second.library, namespace, &session)
        .await
        .unwrap();
    assert!(second.library.get(&next_key).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn read_only_share_reports_typed_failure_without_objects() -> Result<()> {
    let namespace = Uuid::new_v4();
    let library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    store.inject_error(StoreOperation::Put, StoreError::PermissionDenied);
    let session = memory_session(&store).await;
    let result = sync_pass(&library.library, namespace, &session).await;
    assert_eq!(result, Err(ShareSyncFailure::ReadOnlyShare));
    assert!(store.list_prefix("").await.unwrap().is_empty());
    Ok(())
}

#[tokio::test]
async fn read_only_webdav_in_share_probe_reports_typed_state_without_persisting_writes()
-> Result<()> {
    use fsio::test_fixtures::WebDavFixture;

    let server = WebDavFixture::start_read_only(Vec::new()).await?;
    let directory = tempfile::tempdir()?;
    let credentials = CredentialStore::open(directory.path());
    let credential = credentials
        .put(Credential {
            username: "user".into(),
            secret: Secret::new(b"pass".to_vec()),
        })
        .await?;
    let mut record = test_record(Uuid::new_v4());
    record.catalog = CatalogPlacement::InShare(Uuid::new_v4());
    record.fs = vec![FsAdapter::WebDav {
        endpoint: server.endpoint.clone(),
        root: "/dav/library/".into(),
        username: "user".into(),
        credential,
    }];

    let result = OpenDalShareStoreFactory
        .open(&record, "devices/fixture", &credentials)
        .await;
    assert_eq!(
        result.as_ref().err().copied(),
        Some(ShareSyncFailure::ReadOnlyShare),
        "factory returned {:?}; recorded operations: {:?}",
        result.as_ref().err(),
        server.write_requests()
    );
    let refused_operations = server.write_requests();
    assert!(
        !refused_operations.is_empty(),
        "share writability probe did not run"
    );
    assert!(
        refused_operations
            .iter()
            .all(|operation| matches!(operation.as_str(), "PUT" | "DELETE")),
        "unexpected write operation in share probe: {refused_operations:?}"
    );
    Ok(())
}

#[tokio::test]
async fn replaced_library_file_is_a_typed_share_change() -> Result<()> {
    let namespace = Uuid::new_v4();
    let library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    let session = memory_session(&store).await;
    sync_pass(&library.library, namespace, &session)
        .await
        .unwrap();
    let replacement = LibraryFile {
        format: FORMAT_VERSION,
        library_id: Uuid::new_v4(),
        model: RuneModel::model_id().to_owned(),
        schema: RuneModel::schema_id().to_owned(),
    };
    store
        .put(&library_key(), &encode_library(&replacement).unwrap())
        .await
        .unwrap();
    assert_eq!(
        sync_pass(&library.library, namespace, &session).await,
        Err(ShareSyncFailure::ShareChanged)
    );
    assert_eq!(
        store.get(&library_key()).await.unwrap().bytes,
        encode_library(&replacement).unwrap()
    );
    Ok(())
}

#[tokio::test]
async fn cancellation_drops_an_in_flight_share_pass() -> Result<()> {
    let namespace = Uuid::new_v4();
    let library = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    store.configure(MemoryStoreConfig {
        delay: Duration::from_millis(10),
        partial_write_bytes: None,
    });
    let session = memory_session(&store).await;
    let cache = std::env::temp_dir().join(format!("hub-share-{}", Uuid::new_v4()));
    let cancel = CancellationToken::new();
    let library_sync = library.library.clone();
    let session = session.clone();
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        select_share_cancellation(
            task_cancel,
            share_sync_pass(
                &library_sync,
                namespace,
                cache,
                &session,
                SharePass::PullAndPush,
                || {},
            ),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !store.calls().contains(&StoreOperation::Put) {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    cancel.cancel();
    assert!(task.await?.is_none());
    Ok(())
}

#[test]
fn repeated_share_failures_back_off_to_the_cap() {
    let mut previous = None;
    let mut observed = Vec::new();
    for _ in 0..8 {
        let (wait, next) = next_share_backoff(previous);
        observed.push(wait);
        previous = Some(next);
    }
    assert_eq!(
        observed,
        [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(16),
            Duration::from_secs(30),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ]
    );
}
