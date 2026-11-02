//! Exact observed-remove state for one caller-selected namespace and collection.
//!
//! It does not implement paging, persistence, writer fencing, or forgotten-dot provenance.
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Typed failures from causal state operations.
#[derive(Debug)]
pub enum CausalError {
    ConflictingDot,
    ClockExhausted,
    LogicalLimit(&'static str),
    Invalid(String),
}
impl std::fmt::Display for CausalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConflictingDot => f.write_str("conflicting live dot"),
            Self::ClockExhausted => f.write_str("dot counter exhausted"),
            Self::LogicalLimit(message) => f.write_str(message),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for CausalError {}

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
            CausalError::Invalid("invalid dot".into())
        );
        Ok(Self { writer, counter })
    }
    /// Return the next counter without wrapping; lifecycle fencing is caller-owned.
    pub fn next(self) -> Result<Self> {
        Self::new(
            self.writer,
            self.counter
                .checked_add(1)
                .filter(|n| *n <= MAX_DOT_COUNTER)
                .ok_or(CausalError::ClockExhausted)?,
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
    pub fn is_empty(&self) -> bool {
        self.clocks.is_empty()
    }
    pub fn intervals(&self) -> impl Iterator<Item = (Uuid, u64, u64)> + '_ {
        self.clocks.iter().flat_map(|(&writer, clock)| {
            (clock.prefix > 0)
                .then_some((writer, 1, clock.prefix))
                .into_iter()
                .chain(clock.intervals.iter().map(move |&(a, b)| (writer, a, b)))
        })
    }
    pub fn difference(&self, other: &Self) -> Result<Self> {
        let mut result = Self::default();
        let mut rhs = other.intervals().peekable();
        for (writer, start, end) in self.intervals() {
            while rhs
                .peek()
                .is_some_and(|&(w, _, b)| w < writer || (w == writer && b < start))
            {
                rhs.next();
            }
            let mut cursor = start;
            while let Some(&(w, a, b)) = rhs.peek() {
                if w != writer || a > end {
                    break;
                }
                if a > cursor {
                    result.insert_interval(writer, cursor, a - 1)?;
                }
                cursor = cursor.max(b.saturating_add(1));
                if b > end {
                    break;
                }
                rhs.next();
            }
            if cursor <= end {
                result.insert_interval(writer, cursor, end)?;
            }
        }
        Ok(result)
    }
    pub fn is_subset(&self, other: &Self) -> bool {
        let mut rhs = other.intervals().peekable();
        self.intervals().all(|(w, a, b)| {
            while rhs
                .peek()
                .is_some_and(|&(x, _, y)| x < w || (x == w && y < a))
            {
                rhs.next();
            }
            rhs.peek()
                .is_some_and(|&(x, y, z)| x == w && y <= a && z >= b)
        })
    }
    pub fn chunk(&self, after: Option<(Uuid, u64)>, max: usize) -> (Self, Option<(Uuid, u64)>) {
        let mut out = Self::default();
        let mut iter = self
            .intervals()
            .filter(|&(w, a, _)| after.is_none_or(|v| (w, a) > v))
            .peekable();
        let mut last = None;
        for _ in 0..max {
            let Some((w, a, b)) = iter.next() else { break };
            // The intervals are already validated by Context.
            if out.insert_interval(w, a, b).is_err() {
                unreachable!()
            }
            last = Some((w, a));
        }
        (out, if iter.peek().is_some() { last } else { None })
    }
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
            CausalError::Invalid("invalid interval".into())
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
            ensure!(
                a < b,
                CausalError::Invalid("invalid empty or reversed range".into())
            );
        }
        Ok(Self { start, end })
    }
    pub fn contains(&self, k: &K) -> bool {
        self.start.as_ref().is_none_or(|a| a <= k) && self.end.as_ref().is_none_or(|b| k < b)
    }
}
/// Immutable value with caller-defined total-order revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate<V, R> {
    pub dot: Dot,
    pub revision: R,
    pub value: V,
}
/// Explicit in-memory resource limits. These are not a protocol encoding limit.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_live: usize,
    pub max_writers: usize,
    pub max_intervals: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_live: 100_000,
            max_writers: 10_000,
            max_intervals: 100_000,
        }
    }
}
/// Planned candidate additions and observed removals for a complete leaf.
pub type LeafPlan<K, V, R> = (Vec<(K, Candidate<V, R>)>, Context);
/// One collection's accepted causal state.
#[derive(Clone, Debug)]
pub struct CausalMap<K, V, R> {
    base: Context,
    live: BTreeMap<K, BTreeMap<Dot, Candidate<V, R>>>,
    limits: Limits,
}
impl<K: Ord + Clone, V: Clone + Eq, R: Clone + Ord + Eq> CausalMap<K, V, R> {
    pub fn new(limits: Limits) -> Result<Self> {
        ensure!(
            limits.max_live > 0 && limits.max_writers > 0 && limits.max_intervals > 0,
            "limits must be positive"
        );
        Ok(Self {
            base: Context::default(),
            live: BTreeMap::new(),
            limits,
        })
    }
    pub fn live_entries(&self) -> Vec<(K, Candidate<V, R>)> {
        self.live
            .iter()
            .flat_map(|(key, values)| values.values().map(move |c| (key.clone(), c.clone())))
            .collect()
    }
    pub fn live_dots(&self) -> Context {
        let mut context = Context::default();
        for values in self.live.values() {
            for c in values.values() {
                if context.insert(c.dot).is_err() {
                    unreachable!()
                }
            }
        }
        context
    }
    pub fn plan_leaf(
        &self,
        range: &KeyRange<K>,
        incoming: &[(K, Candidate<V, R>)],
        source_base: &Context,
    ) -> Result<LeafPlan<K, V, R>> {
        KeyRange::new(range.start.clone(), range.end.clone())?;
        let mut seen = BTreeSet::new();
        let mut added = Vec::new();
        for (key, candidate) in incoming {
            if !range.contains(key)
                || !source_base.contains(candidate.dot)
                || !seen.insert(candidate.dot)
            {
                return Err(CausalError::Invalid("invalid leaf candidate".into()).into());
            }
            self.check_dot(key, candidate)?;
            if !self
                .live
                .get(key)
                .is_some_and(|v| v.contains_key(&candidate.dot))
                && !self.base().contains(candidate.dot)
            {
                added.push((key.clone(), candidate.clone()));
            }
        }
        let mut observed = Context::default();
        let start = range
            .start
            .as_ref()
            .map_or(std::ops::Bound::Unbounded, std::ops::Bound::Included);
        let end = range
            .end
            .as_ref()
            .map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
        for values in self.live.range((start, end)).map(|(_, values)| values) {
            for dot in values.keys() {
                if !seen.contains(dot) && source_base.contains(*dot) {
                    observed.insert(*dot)?;
                }
            }
        }
        Ok((added, observed))
    }
    pub fn apply_merge(
        &mut self,
        added: &[(K, Candidate<V, R>)],
        observed: &Context,
    ) -> Result<bool> {
        let mut seen = BTreeSet::new();
        for (key, c) in added {
            self.check_dot(key, c)?;
            if !seen.insert(c.dot) || observed.contains(c.dot) {
                return Err(CausalError::Invalid("duplicate or observed added dot".into()).into());
            }
        }
        let mut next = self.clone();
        for values in next.live.values_mut() {
            values.retain(|dot, _| !observed.contains(*dot));
        }
        next.live.retain(|_, values| !values.is_empty());
        for (key, c) in added {
            let live = self.live.get(key).is_some_and(|v| v.contains_key(&c.dot));
            if live || !self.base().contains(c.dot) {
                next.live
                    .entry(key.clone())
                    .or_default()
                    .insert(c.dot, c.clone());
            }
            next.base.insert(c.dot)?;
        }
        next.base = next.base.union(observed)?;
        let changed = self.live != next.live || self.base != next.base;
        if changed {
            next.validate_limits()?;
            *self = next;
        }
        Ok(changed)
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
    /// Caller must include any required deterministic tie-break in R.
    pub fn winner(&self, key: &K) -> Option<&Candidate<V, R>> {
        self.candidates(key)
            .max_by(|a, b| a.revision.cmp(&b.revision).then_with(|| a.dot.cmp(&b.dot)))
    }
    fn validate_limits(&self) -> Result<()> {
        ensure!(
            self.live.values().map(|m| m.len()).sum::<usize>() <= self.limits.max_live,
            CausalError::LogicalLimit("live limit")
        );
        ensure!(
            self.base.clocks.len() <= self.limits.max_writers,
            CausalError::LogicalLimit("writer limit")
        );
        ensure!(
            self.base.interval_count() <= self.limits.max_intervals,
            CausalError::LogicalLimit("interval limit")
        );
        Ok(())
    }
    fn check_dot(&self, key: &K, c: &Candidate<V, R>) -> Result<()> {
        Dot::new(c.dot.writer, c.dot.counter)?;
        for (k, items) in &self.live {
            if let Some(old) = items.get(&c.dot) {
                ensure!(k == key && old == c, CausalError::ConflictingDot);
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
        ensure!(!self.base.contains(candidate.dot), "dot already observed");
        let mut next = self.clone();
        next.remove_observed(&key, observed)?;
        next.base.insert(candidate.dot)?;
        next.live
            .entry(key)
            .or_default()
            .insert(candidate.dot, candidate);
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
            CausalError::LogicalLimit("delta item limit")
        );
        let mut seen = BTreeSet::new();
        for dot in removed {
            Dot::new(dot.writer, dot.counter)?;
            ensure!(seen.insert(*dot), "duplicate removed dot");
            for (live_key, values) in &self.live {
                if live_key != &key && values.contains_key(dot) {
                    return Err(anyhow::Error::new(CausalError::ConflictingDot)
                        .context("removed dot belongs to another live key"));
                }
            }
        }
        for candidate in added {
            self.check_dot(&key, candidate)?;
            ensure!(seen.insert(candidate.dot), "duplicate or removed added dot");
        }
        let before_context = self.base.clone();
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
            let was_observed = self.base().contains(candidate.dot);
            next.base.insert(candidate.dot)?;
            if was_live || !was_observed {
                next.live
                    .entry(key.clone())
                    .or_default()
                    .insert(candidate.dot, candidate.clone());
            }
        }
        let after_context = &next.base;
        let changed = before_live != next.live || &before_context != after_context;
        if changed {
            next.validate_limits()?;
            *self = next;
        }
        Ok(changed)
    }
    /// Compare observable causal state.
    pub fn semantic_eq(&self, other: &Self) -> Result<bool> {
        Ok(self.live == other.live && self.base == other.base)
    }
    /// Merge one complete source atomically.
    pub fn merge(&mut self, other: &Self) -> Result<bool> {
        let entries = other.live_entries();
        let mut incoming = BTreeSet::new();
        for (key, candidate) in &entries {
            self.check_dot(key, candidate)?;
            ensure!(
                other.base.contains(candidate.dot),
                "source candidate not observed"
            );
            ensure!(incoming.insert(candidate.dot), "duplicate source dot");
        }
        self.apply_merge(&entries, &other.base().difference(&other.live_dots())?)
    }
}
