//! In-memory asynchronous object store for share tests.
use super::super::{ObjectKey, parse_key};
use super::{ConditionalGet, ObjectStore, ObjectTag, ObjectValue, StoreError, StoreOperation};
use crate::protocol::BoxFuture;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::poll_fn,
    sync::{Mutex, MutexGuard, PoisonError},
    task::Poll,
    time::Duration,
};

#[derive(Clone, Debug, Default)]
pub struct MemoryStoreConfig {
    /// Number of cooperative executor yields to simulate a slow operation.
    pub delay: Duration,
    pub partial_write_bytes: Option<usize>,
}

#[derive(Default)]
pub struct MemoryObjectStore {
    objects: Mutex<BTreeMap<String, ObjectValue>>,
    failures: Mutex<BTreeMap<StoreOperation, StoreError>>,
    kind_failures: Mutex<BTreeMap<(StoreOperation, StoreObjectKind), StoreError>>,
    config: Mutex<MemoryStoreConfig>,
    calls: Mutex<BTreeSet<StoreOperation>>,
    gets: Mutex<BTreeMap<String, u64>>,
    creates: Mutex<BTreeMap<String, u64>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StoreObjectKind {
    Head,
    Segment,
    Base,
    Chunk,
}

impl MemoryObjectStore {
    pub fn inject_error(&self, operation: StoreOperation, error: StoreError) {
        recover_lock(&self.failures).insert(operation, error);
    }

    pub fn clear_error(&self, operation: StoreOperation) {
        recover_lock(&self.failures).remove(&operation);
    }

    pub fn inject_error_for_kind(
        &self,
        operation: StoreOperation,
        kind: StoreObjectKind,
        error: StoreError,
    ) {
        recover_lock(&self.kind_failures).insert((operation, kind), error);
    }

    pub fn clear_error_for_kind(&self, operation: StoreOperation, kind: StoreObjectKind) {
        recover_lock(&self.kind_failures).remove(&(operation, kind));
    }

    pub fn configure(&self, config: MemoryStoreConfig) {
        *recover_lock(&self.config) = config;
    }

    pub fn calls(&self) -> BTreeSet<StoreOperation> {
        recover_lock(&self.calls).clone()
    }

    pub fn get_count(&self, key: &str) -> u64 {
        recover_lock(&self.gets)
            .get(key)
            .copied()
            .unwrap_or_default()
    }

    pub fn create_count(&self, key: &str) -> u64 {
        recover_lock(&self.creates)
            .get(key)
            .copied()
            .unwrap_or_default()
    }

    fn before<'a>(&'a self, operation: StoreOperation) -> BoxFuture<'a, (), StoreError> {
        Box::pin(async move {
            recover_lock(&self.calls).insert(operation);
            let config = recover_lock(&self.config).clone();
            cooperate(config.delay).await;
            if let Some(error) = recover_lock(&self.failures).get(&operation).cloned() {
                return Err(error);
            }
            Ok(())
        })
    }

    fn before_key<'a>(
        &'a self,
        operation: StoreOperation,
        key: &'a str,
    ) -> BoxFuture<'a, (), StoreError> {
        Box::pin(async move {
            self.before(operation).await?;
            let kind = match parse_key(key) {
                Ok(ObjectKey::Head { .. }) => Some(StoreObjectKind::Head),
                Ok(ObjectKey::Segment { .. }) => Some(StoreObjectKind::Segment),
                Ok(ObjectKey::Base { .. }) => Some(StoreObjectKind::Base),
                Ok(ObjectKey::Chunk { .. }) => Some(StoreObjectKind::Chunk),
                Ok(ObjectKey::Library | ObjectKey::Other(_)) | Err(_) => None,
            };
            if let Some(error) = kind.and_then(|kind| {
                recover_lock(&self.kind_failures)
                    .get(&(operation, kind))
                    .cloned()
            }) {
                return Err(error);
            }
            Ok(())
        })
    }

    fn tag(bytes: &[u8]) -> ObjectTag {
        ObjectTag(blake3::hash(bytes).to_hex().to_string())
    }
}

fn recover_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn cooperate(delay: Duration) {
    let mut yields = delay.as_millis().min(10_000) as u32;
    poll_fn(|context| {
        if yields == 0 {
            Poll::Ready(())
        } else {
            yields -= 1;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await
}

impl ObjectStore for MemoryObjectStore {
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, ObjectValue, StoreError> {
        Box::pin(async move {
            self.before(StoreOperation::Get).await?;
            *recover_lock(&self.gets).entry(key.to_owned()).or_default() += 1;
            recover_lock(&self.objects)
                .get(key)
                .cloned()
                .ok_or(StoreError::NotFound)
        })
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> BoxFuture<'a, ConditionalGet, StoreError> {
        Box::pin(async move {
            self.before(StoreOperation::ConditionalGet).await?;
            let object = recover_lock(&self.objects)
                .get(key)
                .cloned()
                .ok_or(StoreError::NotFound)?;
            if &object.tag == tag {
                Ok(ConditionalGet::NotModified)
            } else {
                Ok(ConditionalGet::Modified(object))
            }
        })
    }

    fn put<'a>(&'a self, key: &'a str, bytes: &'a [u8]) -> BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            self.before_key(StoreOperation::Put, key).await?;
            let config = recover_lock(&self.config).clone();
            let stored = config.partial_write_bytes.map_or_else(
                || bytes.to_vec(),
                |size| bytes[..size.min(bytes.len())].to_vec(),
            );
            let tag = Self::tag(&stored);
            recover_lock(&self.objects).insert(
                key.to_owned(),
                ObjectValue {
                    bytes: stored,
                    tag: tag.clone(),
                },
            );
            if config.partial_write_bytes.is_some() {
                return Err(StoreError::Transient("simulated partial write".into()));
            }
            Ok(tag)
        })
    }

    fn create<'a>(&'a self, key: &'a str, bytes: &'a [u8]) -> BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            self.before_key(StoreOperation::Create, key).await?;
            *recover_lock(&self.creates)
                .entry(key.to_owned())
                .or_default() += 1;
            let mut objects = recover_lock(&self.objects);
            if objects.contains_key(key) {
                return Err(StoreError::PreconditionFailed);
            }
            let tag = Self::tag(bytes);
            objects.insert(
                key.to_owned(),
                ObjectValue {
                    bytes: bytes.to_vec(),
                    tag: tag.clone(),
                },
            );
            Ok(tag)
        })
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            self.before(StoreOperation::ConditionalPut).await?;
            let mut objects = recover_lock(&self.objects);
            let current = objects.get(key).ok_or(StoreError::NotFound)?;
            if &current.tag != tag {
                return Err(StoreError::PreconditionFailed);
            }
            let new_tag = Self::tag(bytes);
            objects.insert(
                key.to_owned(),
                ObjectValue {
                    bytes: bytes.to_vec(),
                    tag: new_tag.clone(),
                },
            );
            Ok(new_tag)
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, (), StoreError> {
        Box::pin(async move {
            self.before(StoreOperation::Delete).await?;
            recover_lock(&self.objects)
                .remove(key)
                .map(|_| ())
                .ok_or(StoreError::NotFound)
        })
    }

    fn list_prefix<'a>(&'a self, prefix: &'a str) -> BoxFuture<'a, Vec<String>, StoreError> {
        Box::pin(async move {
            self.before(StoreOperation::List).await?;
            Ok(recover_lock(&self.objects)
                .keys()
                .filter(|key| key.starts_with(prefix))
                .cloned()
                .collect())
        })
    }
}
