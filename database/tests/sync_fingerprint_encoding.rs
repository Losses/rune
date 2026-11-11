#[path = "support/causal_library.rs"]
mod support;

use anyhow::Result;
use database::sync::{
    LibrarySyncStore,
    model::{Collection, Fingerprint, ObjectKey, Payload, RuneModel},
};
use migration::catalog_identity;
use sea_orm::{
    ConnectionTrait,
    sea_query::{Alias, Expr, ExprTrait, Query},
};
use sync::protocol::{
    self as p, BoxFuture, Candidate, CheckpointKey, Dot, HistoryStatus, ModelId, PageRead,
    PageRequest, ReceiveProgress, Revision, StoreIdentity, SyncModel, SyncPeer,
};
use sync_orm::{causal_store::namespace_model_on, row_policy::BinaryValue};
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
    let target_endpoint = target.library.endpoint();
    let source_endpoint = source.library.endpoint();
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
