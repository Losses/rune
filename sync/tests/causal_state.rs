use anyhow::{Context as _, Result};
use std::collections::BTreeSet;
use sync::causal::{Candidate, CausalMap, Context, Dot, KeyRange, Limits, MAX_DOT_COUNTER};
use uuid::Uuid;
type Map = CausalMap<u32, u32, u64>;
fn dot(actor: u128, n: u64) -> Dot {
    Dot::new(Uuid::from_u128(actor), n).unwrap()
}
fn cand(actor: u128, n: u64, value: u32) -> Candidate<u32, u64> {
    Candidate {
        dot: dot(actor, n),
        revision: n,
        value,
    }
}
fn map() -> Map {
    Map::new(Limits::default()).unwrap()
}
fn rg(start: Option<u32>, end: Option<u32>) -> KeyRange<u32> {
    KeyRange::new(start, end).unwrap()
}
fn context(dots: &[Dot]) -> Context {
    let mut c = Context::default();
    for d in dots {
        c.insert(*d).unwrap();
    }
    c
}
type MapSnapshot = (Vec<(u32, Candidate<u32, u64>)>, Context);
trait Snapshot {
    fn snapshot(&self) -> MapSnapshot;
}
impl Snapshot for Map {
    fn snapshot(&self) -> MapSnapshot {
        (self.live_entries(), self.base().clone())
    }
}
fn merge_leaf(target: &mut Map, source: &Map, range: KeyRange<u32>) -> Result<()> {
    let incoming: Vec<_> = source
        .live_entries()
        .into_iter()
        .filter(|(key, _)| range.contains(key))
        .collect();
    let (added, observed) = target.plan_leaf(&range, &incoming, source.base())?;
    target.apply_merge(&added, &observed)?;
    Ok(())
}
fn seed(s: &mut u64) -> u64 {
    *s = s
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *s
}
#[test]
fn exact_context_matches_dot_oracle_and_boundaries() -> Result<()> {
    let mut seedval = 13;
    let mut a = Context::default();
    let mut b = Context::default();
    let mut set_a = BTreeSet::new();
    let mut set_b = BTreeSet::new();
    for _ in 0..1200 {
        let actor = seed(&mut seedval) % 4 + 1;
        let n = seed(&mut seedval) % 100 + 1;
        let d = dot(actor as u128, n);
        if seed(&mut seedval) & 1 == 0 {
            a.insert(d)?;
            set_a.insert(d);
        } else {
            b.insert(d)?;
            set_b.insert(d);
        }
    }
    let union = a.union(&b)?;
    let oracle: BTreeSet<_> = set_a.union(&set_b).copied().collect();
    for actor in 1..=4 {
        for n in 1..=100 {
            let d = dot(actor, n);
            assert_eq!(union.contains(d), oracle.contains(&d));
        }
    }
    let mut c = Context::default();
    c.insert_interval(Uuid::from_u128(1), 2, MAX_DOT_COUNTER)?;
    assert!(!c.contains(dot(1, 1)));
    c.insert(dot(1, 1))?;
    assert!(c.contains(dot(1, MAX_DOT_COUNTER)));
    let before = c.clone();
    assert!(
        c.insert_interval(Uuid::from_u128(1), 1, MAX_DOT_COUNTER + 1)
            .is_err()
    );
    assert_eq!(before, c);
    assert!(Dot::new(Uuid::nil(), 1).is_err());
    assert!(dot(1, MAX_DOT_COUNTER).next().is_err());
    Ok(())
}
#[test]
fn concurrent_remove_readd_and_immutable_collision() -> Result<()> {
    let mut original = map();
    original.assign(1, cand(1, 1, 10))?;
    let mut remove = original.clone();
    remove.remove(&1);
    let mut edit = original.clone();
    edit.assign(1, cand(2, 1, 20))?;
    remove.merge(&edit)?;
    assert_eq!(remove.winner(&1).unwrap().value, 20);
    remove.remove(&1);
    remove.assign(1, cand(3, 1, 30))?;
    assert_eq!(remove.winner(&1).unwrap().value, 30);
    let old = remove.snapshot();
    assert!(
        remove
            .apply_merge(
                &[(2, cand(4, 1, 5)), (1, cand(3, 1, 999))],
                &Context::default()
            )
            .is_err()
    );
    assert_eq!(old, remove.snapshot());
    Ok(())
}
#[test]
fn receipt_relay_gaps_and_leaf_merges() -> Result<()> {
    let stale = dot(1, 1);
    let unseen = dot(1, 2);
    let mut src = map();
    src.assign(1, cand(1, 1, 10))?;
    src.remove(&1);
    src.assign(8, cand(1, 2, 80))?;
    let mut mid = map();
    // Only source-dead dots can travel without their live evidence.
    let dead = src.base().difference(&src.live_dots())?;
    assert!(mid.apply_merge(&[], &dead)?);
    assert!(!mid.apply_merge(&[], &dead)?);
    assert!(!mid.base().contains(unseen));
    let mut old = map();
    old.assign(1, cand(1, 1, 10))?;
    old.apply_merge(&[], mid.base())?;
    assert!(old.winner(&1).is_none());
    let mut third = map();
    third.assign(1, cand(1, 1, 10))?;
    third.apply_merge(&[], old.base())?;
    assert!(third.winner(&1).is_none());
    let (added, observed) =
        mid.plan_leaf(&rg(Some(5), None), &[(8, cand(1, 2, 80))], src.base())?;
    mid.apply_merge(&added, &observed)?;
    assert_eq!(mid.winner(&8).unwrap().value, 80);
    assert!(mid.base().contains(stale));
    Ok(())
}
#[test]
fn algebra_for_seeded_reachable_states() -> Result<()> {
    const INITIAL_SEED: u64 = 991;
    let mut random = INITIAL_SEED;
    let mut totals = [0usize; 5]; // assignment, removal, nonempty merge, range, collision
    for trial in 0..80 {
        let trace_seed = random;
        let mut states = [map(), map(), map()];
        let mut counters = [1u64; 3];
        let mut covered = [0usize; 5];
        // Distinct writers create real concurrent candidates for the same key.
        for (index, state) in states.iter_mut().enumerate() {
            state.assign(0, cand(index as u128 + 1, 1, index as u32))?;
            covered[0] += 1;
        }
        let concurrent = states[1].clone();
        states[0].merge(&concurrent)?;
        assert_eq!(
            states[0].candidates(&0).count(),
            2,
            "seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed}"
        );
        covered[2] += 1;
        assert!(
            states[2].remove(&0),
            "seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed}"
        );
        covered[1] += 1;
        let source = states[0].clone();
        merge_leaf(&mut states[2], &source, rg(Some(0), Some(1)))?;
        covered[3] += 1;
        // Fixed four high-bit draws per step, independent of the chosen branch.
        for step in 0..48 {
            let writer = (seed(&mut random) >> 32) as usize % 3;
            let key = ((seed(&mut random) >> 32) % 8) as u32;
            let operation = (seed(&mut random) >> 32) % 5;
            let peer = (seed(&mut random) >> 32) as usize % 3;
            match operation {
                0 | 1 => {
                    counters[writer] += 1;
                    states[writer].assign(
                        key,
                        cand(
                            writer as u128 + 1,
                            counters[writer],
                            counters[writer] as u32,
                        ),
                    ).with_context(|| format!("assign seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed} step={step}"))?;
                    covered[0] += 1;
                }
                2 => {
                    states[writer].remove(&key);
                    covered[1] += 1;
                }
                3 => {
                    let other = states[peer].clone();
                    if !other.live_entries().is_empty() {
                        covered[2] += 1;
                    }
                    states[writer].merge(&other).with_context(|| format!("merge seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed} step={step}"))?;
                }
                _ => {
                    let other = states[peer].clone();
                    merge_leaf(&mut states[writer], &other, rg(Some(key), Some(key + 1))).with_context(|| format!("range seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed} step={step}"))?;
                    covered[3] += 1;
                }
            }
        }
        // Reproduce the former progressive-merge defect within every trial.
        let writer = 1000 + trial as u128;
        let mut victim = map();
        victim.assign(1, cand(writer, 1, 10))?;
        let mut conflicting = map();
        conflicting.assign(8, cand(writer, 1, 80))?;
        let before = victim.snapshot();
        assert!(
            victim.merge(&conflicting).is_err(),
            "collision seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed}"
        );
        assert_eq!(
            victim.snapshot(),
            before,
            "collision atomicity seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed}"
        );
        covered[4] += 1;
        assert!(
            covered.iter().all(|count| *count > 0),
            "coverage seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed} counts={covered:?}"
        );
        for i in 0..covered.len() {
            totals[i] += covered[i];
        }
        let mut ab = states[0].clone();
        ab.merge(&states[1])?;
        let mut ba = states[1].clone();
        ba.merge(&states[0])?;
        assert_eq!(
            ab.snapshot(),
            ba.snapshot(),
            "commutative seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed}"
        );
        let mut id = states[0].clone();
        assert!(
            !id.merge(&states[0])?,
            "idempotent seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed}"
        );
        let mut lhs = ab;
        lhs.merge(&states[2])?;
        let mut bc = states[1].clone();
        bc.merge(&states[2])?;
        let mut rhs = states[0].clone();
        rhs.merge(&bc)?;
        assert_eq!(
            lhs.snapshot(),
            rhs.snapshot(),
            "associative seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed}"
        );
    }
    assert!(
        totals[0] > 240 && totals[1] > 80 && totals[2] > 80 && totals[3] > 80 && totals[4] == 80,
        "batch seed={INITIAL_SEED} totals={totals:?}"
    );
    eprintln!(
        "seed={INITIAL_SEED} trials=80 steps_per_trial=48 assignment/remove/nonempty_merge/range/collision={totals:?}"
    );
    Ok(())
}
#[test]
fn leaf_validation_and_resource_failure_are_atomic() -> Result<()> {
    assert!(KeyRange::new(Some(5u32), Some(5)).is_err());
    let mut m = Map::new(Limits {
        max_live: 1,
        ..Limits::default()
    })?;
    m.assign(1, cand(1, 1, 1))?;
    let old = m.snapshot();
    assert!(m.assign(2, cand(2, 1, 2)).is_err());
    assert_eq!(old, m.snapshot());
    assert!(
        m.plan_leaf(
            &rg(Some(2), Some(3)),
            &[(9, cand(4, 1, 1))],
            &context(&[dot(4, 1)])
        )
        .is_err()
    );
    assert_eq!(old, m.snapshot());
    assert!(
        m.apply_merge(&[(2, cand(4, 1, 1))], &Context::default())
            .is_err()
    );
    assert_eq!(old, m.snapshot());
    Ok(())
}
#[test]
fn semantic_equal_merge_chunks_and_steady_state() -> Result<()> {
    let ctx = context(&[dot(9, 1), dot(9, 3)]);
    let mut a = map();
    let mut b = map();
    a.apply_merge(&[], &context(&[dot(9, 1)]))?;
    a.apply_merge(&[], &context(&[dot(9, 3)]))?;
    b.apply_merge(&[], &ctx)?;
    assert_eq!(a.snapshot(), b.snapshot());
    assert!(!a.merge(&b)?);
    assert!(!b.merge(&a)?);
    Ok(())
}
#[test]
fn local_write_between_leaves_survives_and_receipt_is_collection_wide() -> Result<()> {
    let mut s = map();
    let observed = dot(1, 1);
    s.apply_merge(&[], &context(&[observed]))?;
    assert!(s.assign(1, cand(1, 1, 1)).is_err());
    assert!(s.assign(8, cand(1, 1, 1)).is_err());
    assert!(!s.apply_merge(&[(8, cand(1, 1, 1))], &Context::default())?);
    s.assign(1, cand(2, 1, 2))?;
    let (added, removed) = s.plan_leaf(
        &rg(Some(5), None),
        &[(8, cand(1, 1, 1))],
        &context(&[observed]),
    )?;
    assert!(!s.apply_merge(&added, &removed)?);
    assert!(s.winner(&8).is_none());
    assert_eq!(s.winner(&1).unwrap().value, 2);
    Ok(())
}
#[test]
fn merge_chunks_reordered_and_late_stale_relay() -> Result<()> {
    let d = dot(7, 1);
    let suppress = context(&[d]);
    let other = context(&[dot(8, 1)]);
    let mut left = map();
    left.apply_merge(&[], &suppress)?;
    left.apply_merge(&[], &other)?;
    let mut right = map();
    right.apply_merge(&[], &other)?;
    right.apply_merge(&[], &suppress)?;
    assert_eq!(left.snapshot(), right.snapshot());
    assert!(!left.apply_merge(&[(2, cand(7, 1, 7))], &Context::default())?);
    assert!(left.winner(&2).is_none());
    assert!(left.base().contains(d));
    let mut relay = map();
    relay.apply_merge(&[], left.base())?;
    let mut third = map();
    third.assign(2, cand(7, 1, 7))?;
    third.merge(&relay)?;
    assert!(third.winner(&2).is_none());
    Ok(())
}
#[test]
fn concurrent_candidates_are_retained_with_deterministic_winner() -> Result<()> {
    let mut a = map();
    let mut b = map();
    a.assign(4, cand(1, 1, 11))?;
    b.assign(4, cand(2, 2, 22))?;
    a.merge(&b)?;
    assert_eq!(a.candidates(&4).count(), 2);
    assert_eq!(a.winner(&4).unwrap().value, 22);
    let mut c = map();
    c.assign(4, cand(3, 2, 33))?;
    a.merge(&c)?;
    assert_eq!(a.candidates(&4).count(), 3);
    assert_eq!(a.winner(&4).unwrap().value, 33);
    Ok(())
}

#[test]
fn full_merge_checks_all_dots_against_original_receiver_before_removal() -> Result<()> {
    let d = dot(1, 1);
    let mut receiver = map();
    receiver.assign(1, cand(1, 1, 10))?;
    let mut source = map();
    source.assign(8, cand(1, 1, 80))?;
    let before = receiver.snapshot();
    // This type deliberately has no public persistent encoding; the debug
    // snapshot also checks internal state and limits beyond semantic export.
    let internal_before = format!("{receiver:?}").into_bytes();
    assert!(receiver.merge(&source).is_err());
    assert_eq!(receiver.snapshot(), before);
    assert_eq!(format!("{receiver:?}").into_bytes(), internal_before);
    assert_eq!(receiver.winner(&1).unwrap().dot, d);
    Ok(())
}

#[test]
fn full_merge_rejects_same_dot_conflicting_value_or_revision_atomically() -> Result<()> {
    let mut receiver = map();
    receiver.assign(1, cand(1, 1, 10))?;
    for conflict in [
        Candidate {
            dot: dot(1, 1),
            revision: 1,
            value: 11,
        },
        Candidate {
            dot: dot(1, 1),
            revision: 2,
            value: 10,
        },
    ] {
        let mut source = map();
        source.assign(1, conflict)?;
        let before = receiver.snapshot();
        let internal_before = format!("{receiver:?}").into_bytes();
        assert!(receiver.merge(&source).is_err());
        assert_eq!(receiver.snapshot(), before);
        assert_eq!(format!("{receiver:?}").into_bytes(), internal_before);
    }
    Ok(())
}
#[test]
fn explicit_observed_subset_preserves_unobserved_candidates_atomically() -> Result<()> {
    let mut a = map();
    let mut b = map();
    a.assign(1, cand(1, 1, 10))?;
    b.assign(1, cand(2, 1, 20))?;
    a.merge(&b)?;
    let selected = BTreeSet::from([dot(1, 1)]);
    a.assign_observed(1, cand(3, 1, 30), &selected)?;
    let dots: BTreeSet<_> = a.candidates(&1).map(|c| c.dot).collect();
    assert_eq!(dots, BTreeSet::from([dot(2, 1), dot(3, 1)]));
    let before = a.snapshot();
    assert!(
        a.remove_observed(&1, &BTreeSet::from([dot(99, 1)]))
            .is_err()
    );
    assert_eq!(a.snapshot(), before);
    Ok(())
}
#[test]
fn key_delta_context_only_relay_is_idempotent_and_suppresses_late_value() -> Result<()> {
    let mut a = map();
    let d = dot(1, 1);
    assert!(a.merge_key_delta(3, &[d], &[])?);
    assert!(!a.merge_key_delta(3, &[d], &[])?);
    assert!(a.base().contains(d));
    assert!(!a.merge_key_delta(3, &[], &[cand(1, 1, 11)])?);
    assert!(a.winner(&3).is_none());
    let before = a.snapshot();
    assert!(
        a.merge_key_delta(3, &[dot(9, 1)], &[cand(9, 1, 4)])
            .is_err()
    );
    assert_eq!(a.snapshot(), before);
    Ok(())
}

#[test]
fn indexed_snapshot_scan_matches_complete_export() -> Result<()> {
    let mut state = map();
    state.assign(2, cand(1, 1, 10))?;
    state.assign(7, cand(1, 2, 20))?;
    state.assign(3, cand(4, 1, 30))?;
    state.merge_key_delta(9, &[], &[cand(5, 1, 40)])?;
    let mut cursor = None;
    let mut live = Vec::new();
    loop {
        let next = state.next_live(cursor.as_ref().map(|(key, dot)| (key, *dot)), None, None);
        let Some((key, candidate)) = next else { break };
        cursor = Some((key, candidate.dot));
        live.push((key, candidate));
    }
    let expected_live = state.live_entries();
    assert_eq!(live, expected_live);
    Ok(())
}

#[test]
fn additive_context_algebra_and_merge_rejections() -> Result<()> {
    let mut a = Context::default();
    let mut b = Context::default();
    let mut oracle_a = BTreeSet::new();
    let mut oracle_b = BTreeSet::new();
    for actor in 1..=3 {
        for n in 1..=100 {
            let d = dot(actor, n);
            if n % 3 != 0 {
                a.insert(d)?;
                oracle_a.insert(d);
            }
            if n % 4 != 0 {
                b.insert(d)?;
                oracle_b.insert(d);
            }
        }
    }
    a.insert(dot(1, MAX_DOT_COUNTER))?;
    oracle_a.insert(dot(1, MAX_DOT_COUNTER));
    let diff = a.difference(&b)?;
    let expected: BTreeSet<_> = oracle_a.difference(&oracle_b).copied().collect();
    for d in &oracle_a {
        assert_eq!(diff.contains(*d), expected.contains(d));
    }
    assert!(diff.is_subset(&a));
    assert!(Context::default().is_empty());
    let mut cursor = None;
    let mut rebuilt = Context::default();
    loop {
        let (piece, next) = diff.chunk(cursor, 2);
        rebuilt = rebuilt.union(&piece)?;
        if next.is_none() {
            break;
        }
        cursor = next;
    }
    assert_eq!(diff, rebuilt);
    let mut state = map();
    state.assign(1, cand(1, 1, 10))?;
    let before = state.snapshot();
    assert!(
        state
            .apply_merge(&[(2, cand(2, 1, 20))], &context(&[dot(2, 1)]))
            .is_err()
    );
    assert!(
        state
            .apply_merge(
                &[(2, cand(2, 1, 20)), (2, cand(2, 1, 20))],
                &Context::default()
            )
            .is_err()
    );
    assert!(
        state
            .apply_merge(&[(2, cand(1, 1, 20))], &Context::default())
            .is_err()
    );
    assert_eq!(before, state.snapshot());
    Ok(())
}

#[test]
fn session_decomposition_equals_full_join() -> Result<()> {
    let mut random = 991u64;
    let mut skipped = 0usize;
    for trial in 0..120 {
        let mut states = [map(), map(), map()];
        let mut counters = [0u64; 3];
        for _ in 0..48 {
            let i = (seed(&mut random) >> 32) as usize % 3;
            let key = ((seed(&mut random) >> 32) % 8) as u32;
            let op = (seed(&mut random) >> 32) % 4;
            let j = (seed(&mut random) >> 32) as usize % 3;
            match op {
                0 | 1 => {
                    counters[i] += 1;
                    states[i].assign(key, cand(i as u128 + 1, counters[i], key))?;
                }
                2 => {
                    states[i].remove(&key);
                }
                _ => {
                    let other = states[j].clone();
                    states[i].merge(&other)?;
                }
            }
        }
        let source = states[0].clone();
        let snapshot = states[1].clone();
        let third = states[2].clone();
        let mut result = snapshot.clone();
        let mut intermediates = Vec::new();
        let mut source_live = Context::default();
        let interleave = (seed(&mut random) % 5) as usize;
        for i in 0..4 {
            if i == interleave {
                result.merge(&third)?;
            }
            let range = rg(Some(i as u32 * 2), Some(i as u32 * 2 + 2));
            let incoming: Vec<_> = source
                .live_entries()
                .into_iter()
                .filter(|(k, _)| range.contains(k))
                .collect();
            let snapshot_live: Vec<_> = snapshot
                .live_entries()
                .into_iter()
                .filter(|(k, _)| range.contains(k))
                .collect();
            // Equal candidate summaries skip the leaf; its dots never enter D.
            if incoming == snapshot_live {
                skipped += 1;
                intermediates.push(result.clone());
                continue;
            }
            for (_, c) in &incoming {
                source_live.insert(c.dot)?;
            }
            let (added, observed) = result.plan_leaf(&range, &incoming, source.base())?;
            result.apply_merge(&added, &observed)?;
            intermediates.push(result.clone());
        }
        if interleave == 4 {
            result.merge(&third)?;
        }
        let mut arrivals = Context::default();
        for (key, c) in result.live_entries() {
            if !snapshot.candidates(&key).any(|old| old.dot == c.dot)
                && source.base().contains(c.dot)
                && !source_live.contains(c.dot)
            {
                arrivals.insert(c.dot)?;
            }
        }
        result.apply_merge(&[], &arrivals)?;
        let mut after = None;
        loop {
            let (chunk, next) = source.base().chunk(after, 2);
            let observed = chunk.difference(result.base())?;
            result.apply_merge(&[], &observed)?;
            if next.is_none() {
                break;
            }
            after = next;
        }
        let mut expected = snapshot;
        expected.merge(&third)?;
        expected.merge(&source)?;
        assert!(result.semantic_eq(&expected)?, "trial {trial}");
        assert!(!result.clone().merge(&source)?, "trial {trial}");
        for state in intermediates {
            assert!(!expected.clone().merge(&state)?, "trial {trial}");
        }
    }
    assert!(skipped > 0, "no equal range was skipped");
    Ok(())
}
