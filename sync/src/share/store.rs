//! Async object-store contract and typed storage results.
use crate::protocol::BoxFuture;
use std::result::Result as StdResult;
#[path = "memory.rs"]
mod memory;
pub use memory::{MemoryObjectStore, MemoryStoreConfig};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    NotFound,
    PreconditionFailed,
    Transient(String),
    Permanent(String),
}
pub type StoreResult<T> = StdResult<T, StoreError>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectTag(String);

impl ObjectTag {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectValue {
    pub bytes: Vec<u8>,
    pub tag: ObjectTag,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConditionalGet {
    NotModified,
    Modified(ObjectValue),
}

pub trait ObjectStore: Send + Sync {
    fn get<'a>(&'a self, key: &'a str) -> BoxFuture<'a, ObjectValue, StoreError>;
    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> BoxFuture<'a, ConditionalGet, StoreError>;
    fn put<'a>(&'a self, key: &'a str, bytes: &'a [u8]) -> BoxFuture<'a, ObjectTag, StoreError>;
    fn create<'a>(&'a self, key: &'a str, bytes: &'a [u8]) -> BoxFuture<'a, ObjectTag, StoreError>;
    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> BoxFuture<'a, ObjectTag, StoreError>;
    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, (), StoreError>;
    fn list_prefix<'a>(&'a self, prefix: &'a str) -> BoxFuture<'a, Vec<String>, StoreError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StoreOperation {
    Get,
    ConditionalGet,
    Put,
    Create,
    ConditionalPut,
    Delete,
    List,
}
