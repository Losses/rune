//! Exact observed-remove state for one caller-selected namespace and collection.
//!
//! This in-memory primitive accepts only **complete** source ranges. It does
//! not implement paging, persistence, writer fencing, or forgotten-dot provenance.
use anyhow::{Result, bail, ensure};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Counters are limited to the signed database-compatible range.
pub const MAX_DOT_COUNTER: u64 = i64::MAX as u64;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dot {
    pub writer: Uuid,
    pub counter: u64,
}
impl Dot {
    pub fn new(writer: Uuid, counter: u64) -> Result<Self> {
        ensure!(
            !writer.is_nil() && (1..=MAX_DOT_COUNTER).contains(&counter),
            "invalid dot"
        );
        Ok(Self { writer, counter })
    }
    /// Return the next counter without wrapping; lifecycle fencing is caller-owned.
    pub fn next(self) -> Result<Self> {
        Self::new(
            self.writer,
            self.counter
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("dot counter exhausted"))?,
        )
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Clock {
    prefix: u64,
    intervals: Vec<(u64, u64)>,
}
/// Exact dots encoded as per-writer prefix and disjoint nonadjacent intervals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    clocks: BTreeMap<Uuid, Clock>,
}
impl Context {
    pub fn contains(&self, d: Dot) -> bool {
        self.clocks.get(&d.writer).is_some_and(|c| {
            d.counter > 0
                && (d.counter <= c.prefix || {
                    let index = c.intervals.partition_point(|&(lo, _)| lo <= d.counter);
                    index > 0 && d.counter <= c.intervals[index - 1].1
                })
        })
    }
    pub fn insert(&mut self, d: Dot) -> Result<bool> {
        Dot::new(d.writer, d.counter)?;
        self.insert_range(d.writer, d.counter, d.counter)
    }
    pub fn insert_interval(&mut self, writer: Uuid, lo: u64, hi: u64) -> Result<bool> {
        self.insert_range(writer, lo, hi)
    }
    fn insert_range(&mut self, writer: Uuid, lo: u64, hi: u64) -> Result<bool> {
        ensure!(
            !writer.is_nil() && lo > 0 && lo <= hi && hi <= MAX_DOT_COUNTER,
            "invalid interval"
        );
        let clock = self.clocks.entry(writer).or_default();
        if hi <= clock.prefix {
            return Ok(false);
        }
        if let Some(last) = clock.intervals.last_mut() {
            if lo > last.1 {
                if lo == last.1 + 1 {
                    last.1 = hi;
                } else {
                    clock.intervals.push((lo, hi));
                }
                return Ok(true);
            }
        } else if lo > clock.prefix + 1 {
            clock.intervals.push((lo, hi));
            return Ok(true);
        }
        let before = clock.clone();
        let mut all = std::mem::take(&mut clock.intervals);
        all.push((lo, hi));
        all.sort_unstable();
        for (a, b) in all {
            if a <= clock.prefix + 1 {
                clock.prefix = clock.prefix.max(b);
            } else if let Some(last) = clock.intervals.last_mut() {
                if a <= last.1 + 1 {
                    last.1 = last.1.max(b);
                } else {
                    clock.intervals.push((a, b));
                }
            } else {
                clock.intervals.push((a, b));
            }
        }
        Ok(*clock != before)
    }
    pub fn union(&self, other: &Self) -> Result<Self> {
        let mut out = self.clone();
        for (&writer, c) in &other.clocks {
            if c.prefix > 0 {
                out.insert_range(writer, 1, c.prefix)?;
            }
            for &(lo, hi) in &c.intervals {
                out.insert_range(writer, lo, hi)?;
            }
        }
        Ok(out)
    }
    /// Canonical per-writer entries, useful for storage adapters and fixtures.
    pub fn entries(&self) -> impl Iterator<Item = (Uuid, u64, &[(u64, u64)])> {
        self.clocks
            .iter()
            .map(|(&id, c)| (id, c.prefix, c.intervals.as_slice()))
    }
    pub fn interval_count(&self) -> usize {
        self.clocks.values().map(|c| c.intervals.len()).sum()
    }
}
/// Half-open key range; `None` means the corresponding unbounded endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRange<K> {
    pub start: Option<K>,
    pub end: Option<K>,
}
impl<K: Ord> KeyRange<K> {
    pub fn new(start: Option<K>, end: Option<K>) -> Result<Self> {
        if let (Some(a), Some(b)) = (&start, &end) {
            ensure!(a < b, "invalid empty or reversed range");
        }
        Ok(Self { start, end })
    }
    pub fn contains(&self, k: &K) -> bool {
        self.start.as_ref().is_none_or(|a| a <= k) && self.end.as_ref().is_none_or(|b| k < b)
    }
    fn intersects(&self, other: &Self) -> bool {
        self.end
            .as_ref()
            .zip(other.start.as_ref())
            .is_none_or(|(a, b)| a > b)
            && other
                .end
                .as_ref()
                .zip(self.start.as_ref())
                .is_none_or(|(a, b)| a > b)
    }
}
/// Immutable value with caller-defined total-order revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate<V, R> {
    pub dot: Dot,
    pub revision: R,
    pub value: V,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeContext<K> {
    pub range: KeyRange<K>,
    pub context: Context,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteRange<K, V, R> {
    pub range: KeyRange<K>,
    pub context: Context,
    pub candidates: Vec<(K, Candidate<V, R>)>,
}
/// Explicit in-memory resource limits. These are not a protocol encoding limit.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_live: usize,
    pub max_writers: usize,
    pub max_intervals: usize,
    pub max_ranges: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_live: 100_000,
            max_writers: 10_000,
            max_intervals: 100_000,
            max_ranges: 10_000,
        }
    }
}
/// One collection's accepted causal state. Qualified contexts are durable state.
#[derive(Clone, Debug)]
pub struct CausalMap<K, V, R> {
    base: Context,
    live: BTreeMap<K, BTreeMap<Dot, Candidate<V, R>>>,
    ranges: Vec<RangeContext<K>>,
    limits: Limits,
}
impl<K: Ord + Clone, V: Clone + Eq, R: Clone + Ord + Eq> CausalMap<K, V, R> {
    pub fn new(limits: Limits) -> Result<Self> {
        ensure!(
            limits.max_live > 0
                && limits.max_writers > 0
                && limits.max_intervals > 0
                && limits.max_ranges > 0,
            "limits must be positive"
        );
        Ok(Self {
            base: Context::default(),
            live: BTreeMap::new(),
            ranges: Vec::new(),
            limits,
        })
    }
    pub fn base(&self) -> &Context {
        &self.base
    }
    pub fn candidates(&self, key: &K) -> impl Iterator<Item = &Candidate<V, R>> {
        self.live.get(key).into_iter().flat_map(|v| v.values())
    }
    /// Indexed exclusive candidate cursor for bounded snapshot traversal.
    pub fn next_live(
        &self,
        after: Option<(&K, Dot)>,
        lower: Option<&K>,
        upper: Option<&K>,
    ) -> Option<(K, Candidate<V, R>)> {
        use std::ops::Bound::{Excluded, Included, Unbounded};
        let start = match (after.map(|x| x.0), lower) {
            (Some(a), Some(b)) => Included(a.max(b).clone()),
            (Some(a), None) | (None, Some(a)) => Included(a.clone()),
            (None, None) => Unbounded,
        };
        let end = upper.map_or(Unbounded, |key| Excluded(key.clone()));
        for (key, values) in self.live.range((start, end)) {
            let dot_start = match after {
                Some((a, dot)) if a == key => Excluded(dot),
                _ => Unbounded,
            };
            if let Some((_, candidate)) = values.range((dot_start, Unbounded)).next() {
                return Some((key.clone(), candidate.clone()));
            }
        }
        None
    }
    /// Return the exact effective segment starting at or containing `lower`.
    pub fn next_segment(&self, lower: Option<K>) -> RangeContext<K> {
        let index = self
            .ranges
            .partition_point(|item| match (&item.range.start, &lower) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some(a), Some(b)) => a <= b,
            });
        if index > 0 {
            let previous = &self.ranges[index - 1];
            if previous
                .range
                .end
                .as_ref()
                .zip(lower.as_ref())
                .is_none_or(|(end, start)| end > start)
                && !(lower.is_none() && previous.range.start.is_some())
            {
                return RangeContext {
                    range: KeyRange {
                        start: lower,
                        end: previous.range.end.clone(),
                    },
                    context: previous.context.clone(),
                };
            }
        }
        RangeContext {
            range: KeyRange {
                start: lower,
                end: self.ranges.get(index).and_then(|r| r.range.start.clone()),
            },
            context: self.base.clone(),
        }
    }
    /// Caller must include any required deterministic tie-break in R.
    pub fn winner(&self, key: &K) -> Option<&Candidate<V, R>> {
        self.candidates(key)
            .max_by(|a, b| a.revision.cmp(&b.revision).then_with(|| a.dot.cmp(&b.dot)))
    }
    pub fn effective_context(&self, key: &K) -> Result<Context> {
        let mut c = self.base.clone();
        for r in &self.ranges {
            if r.range.contains(key) {
                c = c.union(&r.context)?;
            }
        }
        Ok(c)
    }
    fn validate_limits(&self) -> Result<()> {
        ensure!(
            self.live.values().map(|m| m.len()).sum::<usize>() <= self.limits.max_live,
            "live limit"
        );
        ensure!(self.ranges.len() <= self.limits.max_ranges, "range limit");
        let mut all = self.base.clone();
        for r in &self.ranges {
            all = all.union(&r.context)?;
        }
        ensure!(all.clocks.len() <= self.limits.max_writers, "writer limit");
        let intervals = self
            .ranges
            .iter()
            .try_fold(self.base.interval_count(), |n, r| {
                n.checked_add(r.context.interval_count())
            })
            .ok_or_else(|| anyhow::anyhow!("interval count overflow"))?;
        ensure!(intervals <= self.limits.max_intervals, "interval limit");
        Ok(())
    }
    fn check_dot(&self, key: &K, c: &Candidate<V, R>) -> Result<()> {
        Dot::new(c.dot.writer, c.dot.counter)?;
        for (k, items) in &self.live {
            if let Some(old) = items.get(&c.dot) {
                ensure!(k == key && old == c, "conflicting live dot");
            }
        }
        Ok(())
    }
    /// Assign a freshly allocated dot after observing all current live candidates.
    pub fn assign(&mut self, key: K, candidate: Candidate<V, R>) -> Result<()> {
        let observed: BTreeSet<_> = self.candidates(&key).map(|value| value.dot).collect();
        self.assign_observed(key, candidate, &observed)
    }
    /// Assign while replacing exactly the currently live dots named by the caller.
    pub fn assign_observed(
        &mut self,
        key: K,
        candidate: Candidate<V, R>,
        observed: &BTreeSet<Dot>,
    ) -> Result<()> {
        self.check_dot(&key, &candidate)?;
        ensure!(
            !self.base.contains(candidate.dot)
                && !self
                    .ranges
                    .iter()
                    .any(|range| range.context.contains(candidate.dot)),
            "dot already observed"
        );
        let mut next = self.clone();
        next.remove_observed(&key, observed)?;
        next.base.insert(candidate.dot)?;
        next.live
            .entry(key)
            .or_default()
            .insert(candidate.dot, candidate);
        // Indexed traversal reads the stored disjoint ranges directly. Keep
        // their effective contexts current when the global base grows.
        next.ranges = next
            .normalized()?
            .into_iter()
            .filter(|segment| segment.context != next.base)
            .collect();
        next.validate_limits()?;
        *self = next;
        Ok(())
    }
    pub fn remove(&mut self, key: &K) -> bool {
        self.live.remove(key).is_some()
    }
    /// Remove exactly a validated subset of the key's currently live dots.
    pub fn remove_observed(&mut self, key: &K, observed: &BTreeSet<Dot>) -> Result<bool> {
        let present = self.live.get(key);
        ensure!(
            observed
                .iter()
                .all(|dot| present.is_some_and(|items| items.contains_key(dot))),
            "observed dot is not live at key"
        );
        if let Some(items) = self.live.get_mut(key) {
            for dot in observed {
                items.remove(dot);
            }
            if items.is_empty() {
                self.live.remove(key);
            }
        }
        Ok(!observed.is_empty())
    }
    /// Merge an independently validated key-scoped operation. Removed dots
    /// identify this key; callers must enforce immutable dot provenance and
    /// application authorization before invoking this method.
    pub fn merge_key_delta(
        &mut self,
        key: K,
        removed: &[Dot],
        added: &[Candidate<V, R>],
    ) -> Result<bool> {
        ensure!(
            removed
                .len()
                .checked_add(added.len())
                .is_some_and(|n| n <= self.limits.max_live),
            "delta item limit"
        );
        let mut seen = BTreeSet::new();
        for dot in removed {
            Dot::new(dot.writer, dot.counter)?;
            ensure!(seen.insert(*dot), "duplicate removed dot");
            for (live_key, values) in &self.live {
                ensure!(
                    live_key == &key || !values.contains_key(dot),
                    "removed dot belongs to another live key"
                );
            }
        }
        for candidate in added {
            self.check_dot(&key, candidate)?;
            ensure!(seen.insert(candidate.dot), "duplicate or removed added dot");
        }
        let before_context = self.normalized()?;
        let before_live = self.live.clone();
        let mut next = self.clone();
        for dot in removed {
            next.base.insert(*dot)?;
            if let Some(values) = next.live.get_mut(&key) {
                values.remove(dot);
            }
        }
        next.live.retain(|_, values| !values.is_empty());
        for candidate in added {
            let was_live = self
                .live
                .get(&key)
                .is_some_and(|values| values.contains_key(&candidate.dot));
            let was_observed = self.effective_context(&key)?.contains(candidate.dot);
            next.base.insert(candidate.dot)?;
            if was_live || !was_observed {
                next.live
                    .entry(key.clone())
                    .or_default()
                    .insert(candidate.dot, candidate.clone());
            }
        }
        let after_context = next.normalized()?;
        let changed = before_live != next.live || before_context != after_context;
        if changed {
            next.ranges = after_context
                .into_iter()
                .filter(|segment| segment.context != next.base)
                .collect();
            next.validate_limits()?;
            *self = next;
        }
        Ok(changed)
    }
    /// Merge only a complete source interval; caller validates transport coverage.
    pub fn merge_range(&mut self, source: &CompleteRange<K, V, R>) -> Result<bool> {
        KeyRange::new(source.range.start.clone(), source.range.end.clone())?;
        ensure!(
            source.candidates.len() <= self.limits.max_live,
            "source live limit"
        );
        ensure!(
            source.context.clocks.len() <= self.limits.max_writers,
            "source writer limit"
        );
        ensure!(
            source.context.interval_count() <= self.limits.max_intervals,
            "source interval limit"
        );
        let mut incoming = BTreeMap::<(K, Dot), Candidate<V, R>>::new();
        let mut dots = BTreeMap::<Dot, (K, Candidate<V, R>)>::new();
        for (key, c) in &source.candidates {
            ensure!(source.range.contains(key), "candidate outside range");
            ensure!(
                source.context.contains(c.dot),
                "source candidate not observed"
            );
            self.check_dot(key, c)?;
            if let Some((old_key, old)) = dots.insert(c.dot, (key.clone(), c.clone())) {
                ensure!(old_key == *key && old == *c, "conflicting source dot");
                bail!("duplicate source dot");
            }
            incoming.insert((key.clone(), c.dot), c.clone());
        }
        let before = self.normalized()?;
        let old_live = self.live.clone();
        let mut next = self.clone();
        for (key, items) in &old_live {
            if source.range.contains(key) {
                for &dot in items.keys() {
                    if !incoming.contains_key(&(key.clone(), dot))
                        && source.context.contains(dot)
                        && let Some(row) = next.live.get_mut(key)
                    {
                        row.remove(&dot);
                    }
                }
            }
        }
        next.live.retain(|_, v| !v.is_empty());
        for ((key, dot), c) in incoming {
            let shared = old_live.get(&key).is_some_and(|v| v.contains_key(&dot));
            let observed = self.effective_context(&key)?.contains(dot);
            next.base.insert(dot)?;
            if shared || !observed {
                next.live.entry(key).or_default().insert(dot, c);
            }
        }
        next.ranges.push(RangeContext {
            range: source.range.clone(),
            context: source.context.clone(),
        });
        let after = next.normalized()?;
        let changed = old_live != next.live || before != after;
        if changed {
            next.ranges = after
                .into_iter()
                .filter(|segment| segment.context != next.base)
                .collect();
            next.validate_limits()?;
            *self = next;
        }
        Ok(changed)
    }
    /// Canonical disjoint partition across the whole key space, including empties.
    pub fn normalized(&self) -> Result<Vec<RangeContext<K>>> {
        let mut boundaries = BTreeSet::new();
        for r in &self.ranges {
            if let Some(k) = &r.range.start {
                boundaries.insert(k.clone());
            }
            if let Some(k) = &r.range.end {
                boundaries.insert(k.clone());
            }
        }
        let mut starts = Vec::with_capacity(boundaries.len() + 1);
        starts.push(None);
        starts.extend(boundaries.into_iter().map(Some));
        let mut out: Vec<RangeContext<K>> = Vec::new();
        for i in 0..starts.len() {
            let range = KeyRange {
                start: starts[i].clone(),
                end: starts.get(i + 1).cloned().unwrap_or(None),
            };
            let mut context = self.base.clone();
            for r in &self.ranges {
                if r.range.intersects(&range) {
                    context = context.union(&r.context)?;
                }
            }
            if let Some(last) = out.last_mut()
                && last.context == context
                && last.range.end == range.start
            {
                last.range.end = range.end;
                continue;
            }
            out.push(RangeContext { range, context });
        }
        Ok(out)
    }
    /// Export immutable complete segments suitable for relaying partial state.
    pub fn export(&self) -> Result<Vec<CompleteRange<K, V, R>>> {
        let mut out = Vec::new();
        for segment in self.normalized()? {
            let mut candidates = Vec::new();
            for (key, items) in &self.live {
                if segment.range.contains(key) {
                    for c in items.values() {
                        candidates.push((key.clone(), c.clone()));
                    }
                }
            }
            out.push(CompleteRange {
                range: segment.range,
                context: segment.context,
                candidates,
            });
        }
        Ok(out)
    }
    /// Compare observable causal state independent of receipt partitioning.
    pub fn semantic_eq(&self, other: &Self) -> Result<bool> {
        Ok(self.live == other.live && self.normalized()? == other.normalized()?)
    }
    /// Merge all segments from one complete source, atomically.
    pub fn merge(&mut self, other: &Self) -> Result<bool> {
        let segments = other.export()?;
        // Check the entire incoming view against the original live state before
        // any range can remove a candidate that would expose a dot collision.
        let mut incoming = BTreeMap::<Dot, (K, Candidate<V, R>)>::new();
        for segment in &segments {
            KeyRange::new(segment.range.start.clone(), segment.range.end.clone())?;
            for (key, candidate) in &segment.candidates {
                ensure!(segment.range.contains(key), "candidate outside range");
                ensure!(
                    segment.context.contains(candidate.dot),
                    "source candidate not observed"
                );
                self.check_dot(key, candidate)?;
                ensure!(
                    incoming
                        .insert(candidate.dot, (key.clone(), candidate.clone()))
                        .is_none(),
                    "duplicate source dot"
                );
            }
        }
        let before = self.normalized()?;
        let old = self.live.clone();
        let mut next = self.clone();
        for part in segments {
            next.merge_range(&part)?;
        }
        let changed = old != next.live || before != next.normalized()?;
        if changed {
            *self = next;
        }
        Ok(changed)
    }
}
