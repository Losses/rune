//! Bounded membership probes over accepted context assertions.
//! The caller owns a valid pin for the requested floor and state position.
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, DbErr, ExprTrait, QueryResult,
    sea_query::{Alias, Condition, Expr, Order, Query},
};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ContextScan {
    namespace: Uuid,
    collection: Vec<u8>,
    key_bound: Vec<u8>,
    writer: Uuid,
    counter: i64,
    floor: i64,
    state_seq: i64,
    after: Option<(i64, i64)>,
    baseline_seen: bool,
    basis_generation: Option<Vec<u8>>,
}
#[derive(Clone, Debug)]
pub enum ScanResult {
    Observed,
    Absent,
    Incomplete(ContextScan),
}
fn err(s: &str) -> DbErr {
    DbErr::Custom(s.to_owned())
}
pub(crate) fn active_basis_condition(floor: i64, basis: Vec<u8>) -> Condition {
    Condition::any()
        .add(
            Condition::all()
                .add(Expr::col(Alias::new("state_seq")).eq(floor))
                .add(Expr::col(Alias::new("basis_generation")).eq(basis)),
        )
        .add(
            Condition::all()
                .add(Expr::col(Alias::new("state_seq")).gt(floor))
                .add(Expr::col(Alias::new("basis_generation")).eq(Uuid::nil().as_bytes().to_vec())),
        )
}
async fn validate_floor(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    floor: i64,
) -> Result<Vec<u8>, DbErr> {
    let q = Query::select()
        .columns(["floor_state_seq", "baseline_generation"].map(Alias::new))
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| err("missing causal namespace"))?;
    if row.try_get_by_index::<i64>(0)? != floor {
        return Err(err("stale context scan: floor changed"));
    }
    let basis: Vec<u8> = row.try_get_by_index(1)?;
    if basis.len() != 16 {
        return Err(err("invalid baseline generation"));
    }
    Ok(basis)
}
/// Fetch a bounded page and attach the nearest interval without loading any
/// unrelated interval rows. Results are decoded in order by the caller.
async fn assertion_page_on(
    tx: &DatabaseTransaction,
    scan: &ContextScan,
    basis: &[u8],
    limit: u64,
    only_covering: bool,
) -> Result<Vec<QueryResult>, DbErr> {
    let ranges = Alias::new("context_range");
    let manifest = Alias::new("retained_manifest");
    let intervals = Alias::new("context_interval");
    let manifest_exists = Query::select()
        .expr(Expr::val(1))
        .from_as(Alias::new("sync_manifests"), manifest.clone())
        .and_where(
            Expr::col((manifest.clone(), Alias::new("namespace")))
                .eq(Expr::col((ranges.clone(), Alias::new("namespace")))),
        )
        .and_where(
            Expr::col((manifest.clone(), Alias::new("digest")))
                .eq(Expr::col((ranges.clone(), Alias::new("context_manifest")))),
        )
        .to_owned();
    let nearest = |column: &str| {
        Query::select()
            .column((intervals.clone(), Alias::new(column)))
            .from_as(Alias::new("sync_context_intervals"), intervals.clone())
            .and_where(
                Expr::col((intervals.clone(), Alias::new("namespace")))
                    .eq(Expr::col((ranges.clone(), Alias::new("namespace")))),
            )
            .and_where(
                Expr::col((intervals.clone(), Alias::new("context_manifest")))
                    .eq(Expr::col((ranges.clone(), Alias::new("context_manifest")))),
            )
            .and_where(
                Expr::col((intervals.clone(), Alias::new("writer")))
                    .eq(scan.writer.as_bytes().to_vec()),
            )
            .and_where(Expr::col((intervals.clone(), Alias::new("first"))).lte(scan.counter))
            .order_by((intervals.clone(), Alias::new("first")), Order::Desc)
            .limit(1)
            .to_owned()
    };
    let projected = |value: Expr| -> Expr {
        if only_covering {
            let covers = Condition::all()
                .add(
                    Expr::col((ranges.clone(), Alias::new("lower_bound")))
                        .lte(scan.key_bound.clone()),
                )
                .add(
                    Expr::col((ranges.clone(), Alias::new("upper_bound")))
                        .gt(scan.key_bound.clone()),
                );
            Expr::case(covers, value)
                .finally(Expr::val(Option::<i64>::None))
                .into()
        } else {
            value
        }
    };
    let mut q = Query::select();
    q.columns(
        [
            "state_seq",
            "ordinal",
            "lower_bound",
            "upper_bound",
            "context_manifest",
        ]
        .map(Alias::new),
    )
    .expr_as(
        projected(Expr::exists(manifest_exists)),
        Alias::new("manifest_exists"),
    )
    .expr_as(
        projected(Expr::from(nearest("first"))),
        Alias::new("interval_first"),
    )
    .expr_as(
        projected(Expr::from(nearest("last"))),
        Alias::new("interval_last"),
    )
    .from_as(Alias::new("sync_context_ranges"), ranges)
    .and_where(Expr::col(Alias::new("namespace")).eq(scan.namespace.as_bytes().to_vec()))
    .and_where(Expr::col(Alias::new("collection")).eq(scan.collection.clone()))
    .and_where(Expr::col(Alias::new("state_seq")).gte(scan.floor))
    .and_where(Expr::col(Alias::new("state_seq")).lte(scan.state_seq))
    .cond_where(active_basis_condition(scan.floor, basis.to_vec()));
    if let Some((version, ordinal)) = scan.after {
        q.cond_where(
            Condition::any()
                .add(Expr::col(Alias::new("state_seq")).gt(version))
                .add(
                    Condition::all()
                        .add(Expr::col(Alias::new("state_seq")).eq(version))
                        .add(Expr::col(Alias::new("ordinal")).gt(ordinal)),
                ),
        );
    }
    q.order_by(Alias::new("state_seq"), Order::Asc)
        .order_by(Alias::new("ordinal"), Order::Asc)
        .limit(limit);
    tx.query_all(&q.to_owned()).await
}

impl ContextScan {
    /// The key must use the reviewed finite-bound encoding, not UUID text.
    pub fn new(
        namespace: Uuid,
        collection: Vec<u8>,
        key_bound: Vec<u8>,
        writer: Uuid,
        counter: i64,
        floor: i64,
        state_seq: i64,
    ) -> Result<Self, DbErr> {
        if namespace.is_nil()
            || writer.is_nil()
            || collection.is_empty()
            || collection.len() > 128
            || key_bound.is_empty()
            || counter <= 0
            || floor < 0
            || state_seq < floor
        {
            return Err(err("invalid context probe"));
        }
        Ok(Self {
            namespace,
            collection,
            key_bound,
            writer,
            counter,
            floor,
            state_seq,
            after: None,
            baseline_seen: floor == 0,
            basis_generation: None,
        })
    }
}
/// Probe at most `max_assertions`; exhaustion returns a bound continuation,
/// never false absence. A retained validated manifest is required for each row.
pub async fn scan_on(
    tx: &DatabaseTransaction,
    mut scan: ContextScan,
    max_assertions: usize,
) -> Result<ScanResult, DbErr> {
    if max_assertions == 0 || max_assertions > 1024 {
        return Err(err("invalid context work limit"));
    }
    let basis = validate_floor(tx, scan.namespace, scan.floor).await?;
    if scan
        .basis_generation
        .as_ref()
        .is_some_and(|old| old != &basis)
    {
        return Err(err("stale context scan: baseline generation changed"));
    }
    scan.basis_generation = Some(basis.clone());
    let rows = assertion_page_on(tx, &scan, &basis, max_assertions as u64, true).await?;
    let fetched = rows.len();
    for row in rows {
        let state_seq: i64 = row.try_get_by_index(0)?;
        let ordinal: i64 = row.try_get_by_index(1)?;
        let lower_bound: Vec<u8> = row.try_get_by_index(2)?;
        let upper_bound: Vec<u8> = row.try_get_by_index(3)?;
        let context_manifest: Vec<u8> = row.try_get_by_index(4)?;
        if state_seq < scan.floor
            || ordinal < 0
            || lower_bound >= upper_bound
            || context_manifest.len() != 32
        {
            return Err(err("invalid retained context assertion"));
        }
        if state_seq > scan.floor && !scan.baseline_seen {
            return Err(err("missing baseline coverage"));
        }
        scan.after = Some((state_seq, ordinal));
        if lower_bound <= scan.key_bound && scan.key_bound < upper_bound {
            if scan.floor > 0 && state_seq == scan.floor {
                if scan.baseline_seen {
                    return Err(err("overlapping baseline coverage"));
                }
                scan.baseline_seen = true;
            }
            let manifest_exists: bool = row.try_get_by_index(5)?;
            if !manifest_exists {
                return Err(err("missing retained context manifest"));
            }
            let first: Option<i64> = row.try_get_by_index(6)?;
            let last: Option<i64> = row.try_get_by_index(7)?;
            if let Some(first) = first {
                let last = last.ok_or_else(|| err("invalid retained context interval"))?;
                if first <= 0 || last < first {
                    return Err(err("invalid retained context interval"));
                }
                if last >= scan.counter {
                    return Ok(ScanResult::Observed);
                }
            } else if last.is_some() {
                return Err(err("invalid retained context interval"));
            }
        }
    }
    if fetched < max_assertions {
        if !scan.baseline_seen {
            return Err(err("missing baseline coverage"));
        }
        return Ok(ScanResult::Absent);
    }
    Ok(ScanResult::Incomplete(scan))
}

/// Determine whether exact evidence for a dot covers the whole collection.
/// Qualified assertions may collectively establish that fact; observing a dot
/// only at one key is never treated as global observation. Work overflow is an
/// error, not an inferred absence. Callers must retain a pin for `state_seq`.
pub struct GlobalObservation<'a> {
    pub namespace: Uuid,
    pub collection: &'a [u8],
    pub writer: Uuid,
    pub counter: i64,
    pub floor: i64,
    pub state_seq: i64,
    pub max_assertions: u64,
}

pub async fn globally_observed_on(
    tx: &DatabaseTransaction,
    probe: GlobalObservation<'_>,
) -> Result<bool, DbErr> {
    let GlobalObservation {
        namespace,
        collection,
        writer,
        counter,
        floor,
        state_seq,
        max_assertions,
    } = probe;
    if namespace.is_nil()
        || writer.is_nil()
        || collection.is_empty()
        || collection.len() > 128
        || counter <= 0
        || floor < 0
        || state_seq < floor
        || max_assertions == 0
        || max_assertions > 1024
    {
        return Err(err("invalid global context probe"));
    }
    let basis = validate_floor(tx, namespace, floor).await?;
    let scan = ContextScan::new(
        namespace,
        collection.to_vec(),
        vec![0],
        writer,
        counter,
        floor,
        state_seq,
    )?;
    let assertions = assertion_page_on(tx, &scan, &basis, max_assertions + 1, false).await?;
    // Match the original all-rows header decoding before applying the limit
    // and per-assertion checks; decode the related data only when reached.
    let assertions = assertions
        .into_iter()
        .map(|row| {
            Ok::<_, DbErr>((
                row.try_get_by_index::<i64>(0)?,
                row.try_get_by_index::<i64>(1)?,
                row.try_get_by_index::<Vec<u8>>(2)?,
                row.try_get_by_index::<Vec<u8>>(3)?,
                row.try_get_by_index::<Vec<u8>>(4)?,
                row,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if assertions.len() as u64 > max_assertions {
        return Err(err("global context work limit exceeded"));
    }
    let mut baseline = Vec::new();
    let mut observed = Vec::new();
    for (state_seq, ordinal, lower_bound, upper_bound, context_manifest, row) in assertions {
        if ordinal < 0 || lower_bound >= upper_bound || context_manifest.len() != 32 {
            return Err(err("invalid retained context assertion"));
        }
        if floor > 0 && state_seq == floor {
            baseline.push((lower_bound.clone(), upper_bound.clone()));
        }
        let manifest_exists: bool = row.try_get_by_index(5)?;
        if !manifest_exists {
            return Err(err("missing retained context manifest"));
        }
        let first: Option<i64> = row.try_get_by_index(6)?;
        let last: Option<i64> = row.try_get_by_index(7)?;
        if let Some(first) = first {
            let last = last.ok_or_else(|| err("invalid retained context interval"))?;
            if first <= 0 || last < first {
                return Err(err("invalid retained context interval"));
            }
            if last >= counter {
                observed.push((lower_bound, upper_bound));
            }
        } else if last.is_some() {
            return Err(err("invalid retained context interval"));
        }
    }
    fn covers(mut ranges: Vec<(Vec<u8>, Vec<u8>)>, exact: bool) -> Result<bool, DbErr> {
        ranges.sort();
        let mut end = vec![0_u8];
        for (lo, hi) in ranges {
            if exact && lo != end {
                return Err(err("invalid baseline partition"));
            }
            if lo > end {
                return Ok(false);
            }
            if hi > end {
                end = hi;
            }
        }
        Ok(end == vec![2_u8])
    }
    if floor > 0 && !covers(baseline, true)? {
        return Err(err("missing baseline coverage"));
    }
    covers(observed, false)
}
