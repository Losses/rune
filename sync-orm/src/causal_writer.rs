//! Per-store writer incarnation and monotone reserved-block dot allocation.
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
    reserved_end: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocatedDot {
    pub writer_incarnation: Uuid,
    pub counter: u64,
}
impl WriterIncarnation {
    /// Call once per independently writable handle; clone to share allocation.
    pub fn new(author: Uuid) -> Result<Self, &'static str> {
        Self::with_reservation(author, Uuid::new_v4(), 0, i64::MAX as u64)
    }
    pub(crate) fn with_reservation(
        author: Uuid,
        incarnation: Uuid,
        start: u64,
        end: u64,
    ) -> Result<Self, &'static str> {
        if author.is_nil() || incarnation.is_nil() {
            return Err("writer identity cannot be nil");
        }
        if start >= end || end > i64::MAX as u64 {
            return Err("invalid writer reservation");
        }
        Ok(Self {
            inner: Arc::new(Inner {
                author,
                incarnation,
                counter: AtomicU64::new(start),
                reserved_end: end,
            }),
        })
    }
    pub fn author(&self) -> Uuid {
        self.inner.author
    }
    pub fn id(&self) -> Uuid {
        self.inner.incarnation
    }
    pub fn last_allocated_counter(&self) -> u64 {
        self.inner.counter.load(Ordering::SeqCst)
    }
    /// Allocating consumes a dot even when its subsequent transaction rolls back.
    pub fn allocate(&self) -> Result<AllocatedDot, &'static str> {
        let previous = self
            .inner
            .counter
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < self.inner.reserved_end).then_some(n + 1)
            })
            .map_err(|_| "writer counter reservation exhausted")?;
        Ok(AllocatedDot {
            writer_incarnation: self.inner.incarnation,
            counter: previous + 1,
        })
    }
}
