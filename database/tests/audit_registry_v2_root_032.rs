use std::fs;

use database::connection::StorageMode;
use database::library_registry::{
    LibraryRecordId, LibraryRegistry, create_replica, resolve_library,
};
use fsio::FsIo;
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

const LEGACY_RECORD_ID: &str = "ad8584db-0db5-49e4-b13a-3e80b66d566b";

fn fixture() -> (TempDir, Vec<u8>) {
    let config = TempDir::new().unwrap();
    let old = serde_json::to_vec_pretty(&json!({
        "version": 2,
        "libraries": [{
            "record_id": LEGACY_RECORD_ID,
            "alias": "Legacy control",
            "kind": { "type": "rune_remote_control", "fingerprint": "legacy-peer" }
        }],
        "last_open_record": null
    }))
    .unwrap();
    fs::write(config.path().join("libraries.json"), &old).unwrap();
    (config, old)
}

fn json_file(path: impl AsRef<std::path::Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn assert_v2_converted(config: &TempDir, old: &[u8]) {
    assert_eq!(
        json_file(config.path().join("libraries.json"))["version"],
        3
    );
    assert_eq!(
        fs::read(config.path().join("libraries.json.v2.bak")).unwrap(),
        old
    );
}

fn assert_legacy_record_preserved(registry: &LibraryRegistry) {
    let legacy_id = LibraryRecordId(Uuid::parse_str(LEGACY_RECORD_ID).unwrap());
    assert!(
        registry
            .libraries
            .iter()
            .any(|record| record.record_id == legacy_id && record.alias == "Legacy control"),
        "the converted registry dropped the existing record"
    );
}

#[tokio::test]
async fn local_v2_direct_registration_preserves_registry() {
    let (config, old) = fixture();
    let media = TempDir::new().unwrap();
    let fsio = FsIo::new();
    let mut registry = LibraryRegistry::load(config.path()).unwrap();
    assert_eq!(registry.libraries.len(), 1);

    let record = resolve_library(
        &fsio,
        &mut registry,
        media.path().to_str().unwrap(),
        "Local",
        StorageMode::Portable,
    )
    .await
    .expect("v2 local registration should convert the registry and succeed");

    assert_eq!(record.alias, "Local");
    assert_legacy_record_preserved(&registry);
    assert_eq!(registry.libraries.len(), 2);
    assert_v2_converted(&config, &old);

    let descriptor_path = media.path().join(".rune/library.json");
    let descriptor = json_file(&descriptor_path);
    let library_id = record.catalog_library_id().unwrap();
    assert_eq!(descriptor["library_id"], library_id.to_string());
    let reloaded = LibraryRegistry::load(config.path()).unwrap();
    assert_legacy_record_preserved(&reloaded);
    assert_eq!(reloaded.libraries.len(), 2);

    // Registering the same folder again keeps the descriptor's identity.
    let reopened = resolve_library(
        &fsio,
        &mut registry,
        media.path().to_str().unwrap(),
        "Local",
        StorageMode::Portable,
    )
    .await
    .unwrap();
    assert_eq!(reopened.catalog_library_id(), Some(library_id));
    assert_eq!(reopened.record_id, record.record_id);
    assert_eq!(json_file(&descriptor_path), descriptor);
    let reloaded = LibraryRegistry::load(config.path()).unwrap();
    assert_legacy_record_preserved(&reloaded);
    assert_eq!(reloaded.libraries.len(), 2);
}

#[tokio::test]
async fn new_replica_v2_direct_registration_preserves_registry() {
    let (config, old) = fixture();
    let mut registry = LibraryRegistry::load(config.path()).unwrap();
    let id = Uuid::new_v4();

    let record = create_replica(config.path(), &mut registry, id, "Remote", "owner")
        .await
        .expect("v2 replica registration should convert the registry and succeed");

    assert_eq!(record.catalog_library_id(), Some(id));
    assert_eq!(record.alias, "Remote");
    assert_legacy_record_preserved(&registry);
    assert_eq!(registry.libraries.len(), 2);
    assert_v2_converted(&config, &old);

    let descriptor_path = config
        .path()
        .join("replicas")
        .join(id.to_string())
        .join(".rune/library.json");
    let descriptor = json_file(&descriptor_path);
    assert_eq!(descriptor["library_id"], id.to_string());
    assert_eq!(descriptor["authority"]["fingerprint"], "owner");

    // Registering the same replica again is idempotent and keeps its identity.
    let again = create_replica(config.path(), &mut registry, id, "Changed", "owner")
        .await
        .unwrap();
    assert_eq!(again, record);
    assert_eq!(json_file(&descriptor_path), descriptor);
    let reloaded = LibraryRegistry::load(config.path()).unwrap();
    assert_legacy_record_preserved(&reloaded);
    assert_eq!(reloaded.libraries.len(), 2);
}

#[tokio::test]
async fn v3_direct_registration_and_existing_replica_control() {
    let config = TempDir::new().unwrap();
    fs::write(
        config.path().join("libraries.json"),
        br#"{"version":3,"libraries":[],"last_open_record":null}"#,
    )
    .unwrap();
    let media = TempDir::new().unwrap();
    let mut registry = LibraryRegistry::load(config.path()).unwrap();
    let local = resolve_library(
        &FsIo::new(),
        &mut registry,
        media.path().to_str().unwrap(),
        "Local",
        StorageMode::Portable,
    )
    .await
    .unwrap();
    assert!(local.catalog_library_id().is_some());
    let id = Uuid::new_v4();
    let first = create_replica(config.path(), &mut registry, id, "Remote", "owner")
        .await
        .unwrap();
    let again = create_replica(config.path(), &mut registry, id, "Changed", "owner")
        .await
        .unwrap();
    assert_eq!(first, again);
    assert_eq!(
        LibraryRegistry::load(config.path())
            .unwrap()
            .libraries
            .len(),
        2
    );
}
