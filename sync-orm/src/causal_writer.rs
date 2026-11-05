//! Per-handle writer incarnation and monotone in-process dot allocation.
//! This is separate from the caller's original revision author.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct WriterIncarnation {
    inner: Arc<Inner>,
}
#[derive(Debug)]
struct Inner {
    author: Uuid,
    incarnation: Uuid,
    counter: AtomicU64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocatedDot {
    pub writer_incarnation: Uuid,
    pub counter: u64,
}
impl WriterIncarnation {
    /// Call once per independently writable handle; clone to share allocation.
    pub fn new(author: Uuid) -> Result<Self, &'static str> {
        if author.is_nil() {
            return Err("author cannot be nil");
        }
        Ok(Self {
            inner: Arc::new(Inner {
                author,
                incarnation: Uuid::new_v4(),
                counter: AtomicU64::new(0),
            }),
        })
    }
    pub fn author(&self) -> Uuid {
        self.inner.author
    }
    pub fn id(&self) -> Uuid {
        self.inner.incarnation
    }
    /// Allocating consumes a dot even when its subsequent transaction rolls back.
    pub fn allocate(&self) -> Result<AllocatedDot, &'static str> {
        let previous = self
            .inner
            .counter
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < i64::MAX as u64).then_some(n + 1)
            })
            .map_err(|_| "writer counter exhausted")?;
        Ok(AllocatedDot {
            writer_incarnation: self.inner.incarnation,
            counter: previous + 1,
        })
    }
}
