use anyhow::{Context as _, Result};
use std::collections::BTreeSet;
use sync::causal::{
    Candidate, CausalMap, CompleteRange, Context, Dot, KeyRange, Limits, MAX_DOT_COUNTER,
};
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
    let old = remove.export()?;
    let bad = CompleteRange {
        range: rg(None, None),
        context: context(&[dot(3, 1), dot(4, 1)]),
        candidates: vec![(2, cand(4, 1, 5)), (1, cand(3, 1, 999))],
    };
    assert!(remove.merge_range(&bad).is_err());
    assert_eq!(old, remove.export()?);
    Ok(())
}
#[test]
fn receipt_relay_gaps_and_ranges() -> Result<()> {
    let stale = dot(1, 1);
    let unseen = dot(1, 2);
    let mut src = map();
    src.assign(1, cand(1, 1, 10))?;
    src.remove(&1);
    src.assign(8, cand(1, 2, 80))?;
    let mut mid = map();
    let first = CompleteRange {
        range: rg(None, Some(5)),
        context: src.effective_context(&1)?,
        candidates: vec![],
    };
    assert!(mid.merge_range(&first)?);
    assert!(!mid.merge_range(&first)?);
    let mut old = map();
    old.assign(1, cand(1, 1, 10))?;
    for p in mid.export()? {
        old.merge_range(&p)?;
    }
    assert!(old.winner(&1).is_none());
    let mut third = map();
    third.assign(1, cand(1, 1, 10))?;
    for p in old.export()? {
        third.merge_range(&p)?;
    }
    assert!(third.winner(&1).is_none());
    let part = CompleteRange {
        range: rg(Some(5), None),
        context: context(&[stale, unseen]),
        candidates: vec![(8, cand(1, 2, 80))],
    };
    mid.merge_range(&part)?;
    assert_eq!(mid.winner(&8).unwrap().value, 80);
    assert!(mid.effective_context(&1)?.contains(stale));
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
        let segment = CompleteRange {
            range: rg(Some(0), Some(1)),
            context: states[0].effective_context(&0)?,
            candidates: states[0]
                .candidates(&0)
                .cloned()
                .map(|candidate| (0, candidate))
                .collect(),
        };
        states[2].merge_range(&segment)?;
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
                    if !other
                        .export()?
                        .iter()
                        .all(|part| part.candidates.is_empty())
                    {
                        covered[2] += 1;
                    }
                    states[writer].merge(&other).with_context(|| format!("merge seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed} step={step}"))?;
                }
                _ => {
                    let other = states[peer].clone();
                    let segment = CompleteRange {
                        range: rg(Some(key), Some(key + 1)),
                        context: other.effective_context(&key)?,
                        candidates: other
                            .candidates(&key)
                            .cloned()
                            .map(|candidate| (key, candidate))
                            .collect(),
                    };
                    states[writer].merge_range(&segment).with_context(|| format!("range seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed} step={step}"))?;
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
        conflicting.merge_range(&CompleteRange {
            range: rg(None, Some(5)),
            context: context(&[dot(writer + 1000, 1)]),
            candidates: vec![],
        })?;
        let before = victim.export()?;
        assert!(
            victim.merge(&conflicting).is_err(),
            "collision seed={INITIAL_SEED} trial={trial} trace_seed={trace_seed}"
        );
        assert_eq!(
            victim.export()?,
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
            ab.export()?,
            ba.export()?,
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
            lhs.export()?,
            rhs.export()?,
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
fn range_validation_and_resource_failure_are_atomic() -> Result<()> {
    assert!(KeyRange::new(Some(5u32), Some(5)).is_err());
    let mut m = Map::new(Limits {
        max_live: 1,
        ..Limits::default()
    })?;
    m.assign(1, cand(1, 1, 1))?;
    let old = m.export()?;
    assert!(m.assign(2, cand(2, 1, 2)).is_err());
    assert_eq!(old, m.export()?);
    let bad = CompleteRange {
        range: rg(Some(2), Some(3)),
        context: context(&[dot(4, 1)]),
        candidates: vec![(9, cand(4, 1, 1))],
    };
    assert!(m.merge_range(&bad).is_err());
    assert_eq!(old, m.export()?);
    Ok(())
}
#[test]
fn semantic_equal_partitions_and_steady_state() -> Result<()> {
    let ctx = context(&[dot(9, 1), dot(9, 3)]);
    let mut a = map();
    let mut b = map();
    a.merge_range(&CompleteRange {
        range: rg(None, Some(5)),
        context: ctx.clone(),
        candidates: vec![],
    })?;
    a.merge_range(&CompleteRange {
        range: rg(Some(5), None),
        context: ctx.clone(),
        candidates: vec![],
    })?;
    b.merge_range(&CompleteRange {
        range: rg(None, None),
        context: ctx,
        candidates: vec![],
    })?;
    assert_eq!(a.export()?, b.export()?);
    assert!(!a.merge(&b)?);
    assert!(!b.merge(&a)?);
    Ok(())
}
#[test]
fn local_write_between_ranges_survives_and_receipt_is_qualified() -> Result<()> {
    let mut s = map();
    let observed = dot(1, 1);
    let part = CompleteRange {
        range: rg(None, Some(5)),
        context: context(&[observed]),
        candidates: vec![],
    };
    s.merge_range(&part)?;
    assert!(s.assign(1, cand(1, 1, 1)).is_err());
    assert!(s.assign(8, cand(1, 1, 1)).is_err());
    // A relayed existing dot remains valid outside the qualified range.
    s.merge_range(&CompleteRange {
        range: rg(Some(8), Some(9)),
        context: context(&[observed]),
        candidates: vec![(8, cand(1, 1, 1))],
    })?;
    s.assign(1, cand(2, 1, 2))?;
    let later = CompleteRange {
        range: rg(Some(5), None),
        context: context(&[observed]),
        candidates: vec![(8, cand(1, 1, 1))],
    };
    s.merge_range(&later)?;
    assert_eq!(s.winner(&8).unwrap().value, 1);
    assert_eq!(s.winner(&1).unwrap().value, 2);
    Ok(())
}
#[test]
fn overlapping_ranges_reordered_and_late_stale_relay() -> Result<()> {
    let d = dot(7, 1);
    let c = Candidate {
        dot: d,
        revision: 1,
        value: 7,
    };
    let suppress = CompleteRange {
        range: rg(None, Some(7)),
        context: context(&[d]),
        candidates: vec![],
    };
    let other = CompleteRange {
        range: rg(Some(3), None),
        context: context(&[dot(8, 1)]),
        candidates: vec![],
    };
    let mut left = map();
    left.merge_range(&suppress)?;
    left.merge_range(&other)?;
    let mut right = map();
    right.merge_range(&other)?;
    right.merge_range(&suppress)?;
    assert_eq!(left.export()?, right.export()?);
    let stale = CompleteRange {
        range: rg(Some(2), Some(3)),
        context: context(&[d]),
        candidates: vec![(2, c)],
    };
    left.merge_range(&stale)?;
    assert!(left.winner(&2).is_none());
    assert!(left.base().contains(d));
    let mut relay = map();
    for segment in left.export()? {
        relay.merge_range(&segment)?;
    }
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
    // Force the source to export the first interval without key 8. Its base
    // context contains d, so a progressive merge would first remove key 1.
    source.merge_range(&CompleteRange {
        range: rg(None, Some(5)),
        context: context(&[dot(2, 1)]),
        candidates: vec![],
    })?;
    assert!(source.export()?.len() > 1);
    let before = receiver.export()?;
    // This type deliberately has no public persistent encoding; the debug
    // snapshot also checks internal state and limits beyond semantic export.
    let internal_before = format!("{receiver:?}").into_bytes();
    assert!(receiver.merge(&source).is_err());
    assert_eq!(receiver.export()?, before);
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
        let before = receiver.export()?;
        let internal_before = format!("{receiver:?}").into_bytes();
        assert!(receiver.merge(&source).is_err());
        assert_eq!(receiver.export()?, before);
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
    let before = a.export()?;
    assert!(
        a.remove_observed(&1, &BTreeSet::from([dot(99, 1)]))
            .is_err()
    );
    assert_eq!(a.export()?, before);
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
    let before = a.export()?;
    assert!(
        a.merge_key_delta(3, &[dot(9, 1)], &[cand(9, 1, 4)])
            .is_err()
    );
    assert_eq!(a.export()?, before);
    Ok(())
}

#[test]
fn indexed_snapshot_scan_matches_complete_export() -> Result<()> {
    let mut state = map();
    state.assign(2, cand(1, 1, 10))?;
    state.assign(7, cand(1, 2, 20))?;
    state.merge_range(&CompleteRange {
        range: rg(None, Some(5)),
        context: context(&[dot(2, 4)]),
        candidates: vec![],
    })?;
    state.merge_range(&CompleteRange {
        range: rg(Some(8), Some(12)),
        context: context(&[dot(3, 3)]),
        candidates: vec![],
    })?;
    // Both local assignments and relayed deltas grow the base after range
    // knowledge has arrived. Indexed contexts must include the new live dots.
    state.assign(3, cand(4, 1, 30))?;
    state.merge_key_delta(9, &[], &[cand(5, 1, 40)])?;
    let expected = state.normalized()?;
    let mut actual = Vec::new();
    let mut lower = None;
    loop {
        let segment = state.next_segment(lower);
        let end = segment.range.end;
        actual.push(segment);
        if end.is_none() {
            break;
        }
        lower = end;
    }
    assert_eq!(actual, expected);
    let mut cursor = None;
    let mut live = Vec::new();
    loop {
        let next = state.next_live(cursor.as_ref().map(|(key, dot)| (key, *dot)), None, None);
        let Some((key, candidate)) = next else { break };
        cursor = Some((key, candidate.dot));
        live.push((key, candidate));
    }
    let expected_live: Vec<_> = state
        .export()?
        .into_iter()
        .flat_map(|s| s.candidates)
        .collect();
    assert_eq!(live, expected_live);
    Ok(())
}
