use fsio::{OpenDalConfig, OpenDalObjectStore};
use sync::share::{ConditionalGet, ObjectStore, StoreError};

#[path = "../src/test_fixtures.rs"]
mod test_fixtures;
use test_fixtures::{S3Fixture, WebDavFixture};

async fn assert_probed_conditional_effects(
    store: &OpenDalObjectStore,
    caps: fsio::ObjectStoreCapabilities,
) {
    let create_key = "devices/probe-outcomes/create";
    if caps.conditional_create {
        store.create(create_key, b"first").await.unwrap();
        assert!(matches!(
            store.create(create_key, b"second").await,
            Err(StoreError::PreconditionFailed)
        ));
        store.delete(create_key).await.unwrap();
    }

    let put_key = "devices/probe-outcomes/put";
    if caps.conditional_put {
        store.put(put_key, b"initial").await.unwrap();
        let fresh = store.get(put_key).await.unwrap().tag;
        store
            .put_if_match(put_key, &fresh, b"updated")
            .await
            .unwrap();
        assert!(matches!(
            store.put_if_match(put_key, &fresh, b"stale").await,
            Err(StoreError::PreconditionFailed)
        ));
        store.delete(put_key).await.unwrap();
    }
}

fn webdav_config(server: &WebDavFixture) -> OpenDalConfig {
    OpenDalConfig::WebDav {
        endpoint: server.endpoint.clone(),
        root: "/dav/library/".into(),
        username: "user".into(),
        password: "pass".into(),
    }
}

fn s3_config(server: &S3Fixture) -> OpenDalConfig {
    OpenDalConfig::S3 {
        endpoint: server.endpoint.clone(),
        bucket: "bucket".into(),
        region: "us-east-1".into(),
        root: "/library/".into(),
        access_key_id: "access".into(),
        secret_access_key: "secret".into(),
    }
}

#[tokio::test]
async fn webdav_probe_and_tag_roundtrip() {
    let server = WebDavFixture::start(Vec::new()).await.unwrap();
    let store = OpenDalObjectStore::new(webdav_config(&server)).unwrap();
    let caps = fsio::self_test::probe_share_object_store(&store, "devices/test.generation")
        .await
        .unwrap();
    assert!(caps.write_probe);
    assert_probed_conditional_effects(&store, caps).await;
    let tag = store
        .put("devices/test.generation/value", b"value")
        .await
        .unwrap();
    let object = store.get("devices/test.generation/value").await.unwrap();
    assert_eq!(object.bytes, b"value");
    assert_eq!(object.tag, tag);
    match store
        .get_if_none_match("devices/test.generation/value", &tag)
        .await
    {
        Ok(ConditionalGet::NotModified) => assert!(caps.conditional_get),
        Ok(ConditionalGet::Modified(value)) => assert_eq!(value.bytes, b"value"),
        Err(_) => assert!(!caps.conditional_get),
    }
    store.delete("devices/test.generation/value").await.unwrap();
    store.delete("devices/test.generation/value").await.unwrap();
}

#[tokio::test]
async fn s3_probe_and_tag_roundtrip() {
    let server = S3Fixture::start(Vec::new()).await.unwrap();
    let store = OpenDalObjectStore::new(s3_config(&server)).unwrap();
    let caps = fsio::self_test::probe_share_object_store(&store, "devices/test.generation")
        .await
        .unwrap();
    assert!(caps.write_probe);
    assert_probed_conditional_effects(&store, caps).await;
    let tag = store
        .put("devices/test.generation/value", b"value")
        .await
        .unwrap();
    let object = store.get("devices/test.generation/value").await.unwrap();
    assert_eq!(object.bytes, b"value");
    assert_eq!(object.tag, tag);
    match store
        .get_if_none_match("devices/test.generation/value", &tag)
        .await
    {
        Ok(ConditionalGet::NotModified) => assert!(caps.conditional_get),
        Ok(ConditionalGet::Modified(value)) => assert_eq!(value.bytes, b"value"),
        Err(_) => assert!(!caps.conditional_get),
    }
    store.delete("devices/test.generation/value").await.unwrap();
    store.delete("devices/test.generation/value").await.unwrap();
}

#[tokio::test]
async fn conditional_if_match_probe_disables_ignored_and_rejected_headers() {
    for mode in ["ignored", "rejected"] {
        let server = WebDavFixture::start(Vec::new()).await.unwrap();
        if mode == "ignored" {
            server.ignore_if_match();
        } else {
            server.reject_if_match_with_bad_request();
        }
        let store = OpenDalObjectStore::new(webdav_config(&server)).unwrap();
        let caps = fsio::self_test::probe_share_object_store(&store, "devices/test.generation")
            .await
            .unwrap();
        assert!(
            caps.write_probe,
            "{mode} conditional behavior must not disable sync writes"
        );
        assert!(!caps.conditional_put, "{mode} If-Match must be disabled");
        assert!(server.write_requests().contains(&"PUT".to_string()));
    }
}

#[tokio::test]
async fn failed_roundtrip_probe_disables_the_share_store() {
    let server = WebDavFixture::start(Vec::new()).await.unwrap();
    server.reject_reads();
    let store = OpenDalObjectStore::new(webdav_config(&server)).unwrap();
    assert!(
        fsio::self_test::probe_share_object_store(&store, "devices/test.generation")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn adapter_rejects_keys_outside_object_namespace() {
    let server = WebDavFixture::start(Vec::new()).await.unwrap();
    let store = OpenDalObjectStore::new(webdav_config(&server)).unwrap();
    assert!(matches!(
        store.put("../outside", b"x").await,
        Err(StoreError::Permanent(_))
    ));
    assert!(matches!(
        store.probe("elsewhere/device").await,
        Err(StoreError::Permanent(_))
    ));
}
