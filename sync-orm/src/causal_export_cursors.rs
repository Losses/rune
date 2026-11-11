//! Durable local journal positions retained for external export targets.
use crate::causal_leases::lock_on;
use anyhow::{Result, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait, FromQueryResult,
    sea_query::{Alias, Expr, Func, JoinType, Query},
};
use uuid::Uuid;

/// Maximum age for an export pin before the exporter must publish a base.
pub const EXPORT_CURSOR_MAX_AGE_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// Maximum retained journal data behind one export cursor.
pub const EXPORT_CURSOR_MAX_RETAINED_BYTES: i64 = 64 * 1024 * 1024;
const MAX_EXPORT_ID_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, FromQueryResult)]
pub struct ExportCursor {
    pub export_id: Vec<u8>,
    pub sequence: i64,
    pub advanced_at_ms: i64,
    pub needs_base: i64,
}

impl ExportCursor {
    pub fn needs_base(&self) -> bool {
        self.needs_base != 0
    }
}

fn namespace(ns: Uuid) -> Vec<u8> {
    ns.as_bytes().to_vec()
}

fn validate_id(export_id: &[u8]) -> Result<()> {
    ensure!(
        !export_id.is_empty() && export_id.len() <= MAX_EXPORT_ID_BYTES,
        "invalid export cursor id"
    );
    Ok(())
}

async fn position_on(tx: &DatabaseTransaction, ns: Uuid) -> Result<(i64, i64, i64)> {
    let row = tx
        .query_one(
            &Query::select()
                .columns(["head", "history_floor", "floor_state_seq"].map(Alias::new))
                .from(Alias::new("sync_meta"))
                .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
                .to_owned(),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing causal namespace"))?;
    Ok((
        row.try_get_by_index(0)?,
        row.try_get_by_index(1)?,
        row.try_get_by_index(2)?,
    ))
}

/// Create a cursor at a known local journal position in the caller's transaction.
pub async fn create_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    export_id: &[u8],
    sequence: i64,
    now_ms: i64,
) -> Result<()> {
    lock_on(tx, ns).await?;
    validate_id(export_id)?;
    ensure!(
        sequence >= 0 && now_ms >= 0,
        "invalid export cursor position"
    );
    let (head, floor, _) = position_on(tx, ns).await?;
    ensure!(sequence <= head, "export cursor is ahead of the journal");
    let insert = Query::insert()
        .into_table(Alias::new("sync_export_cursors"))
        .columns(
            [
                "namespace",
                "export_id",
                "sequence",
                "advanced_at_ms",
                "needs_base",
            ]
            .map(Alias::new),
        )
        .values_panic([
            namespace(ns).into(),
            export_id.to_vec().into(),
            sequence.into(),
            now_ms.into(),
            i64::from(sequence < floor).into(),
        ])
        .to_owned();
    tx.execute(&insert).await?;
    Ok(())
}

/// Advance after the export head is confirmed; the caller commits this with its local work.
pub async fn advance_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    export_id: &[u8],
    sequence: i64,
    now_ms: i64,
) -> Result<()> {
    lock_on(tx, ns).await?;
    validate_id(export_id)?;
    ensure!(
        sequence >= 0 && now_ms >= 0,
        "invalid export cursor position"
    );
    let (head, _, _) = position_on(tx, ns).await?;
    ensure!(sequence <= head, "export cursor is ahead of the journal");
    let changed = tx
        .execute(
            &Query::update()
                .table(Alias::new("sync_export_cursors"))
                .value(Alias::new("sequence"), sequence)
                .value(Alias::new("advanced_at_ms"), now_ms)
                .value(Alias::new("needs_base"), 0_i64)
                .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
                .and_where(Expr::col(Alias::new("export_id")).eq(export_id.to_vec()))
                .and_where(Expr::col(Alias::new("sequence")).lte(sequence))
                .to_owned(),
        )
        .await?
        .rows_affected();
    ensure!(changed == 1, "missing or backwards export cursor advance");
    Ok(())
}

/// Read the persisted export position in the caller's transaction.
pub async fn read_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    export_id: &[u8],
) -> Result<Option<ExportCursor>> {
    validate_id(export_id)?;
    Ok(ExportCursor::find_by_statement(
        tx.get_database_backend().build(
            &Query::select()
                .columns(["export_id", "sequence", "advanced_at_ms", "needs_base"].map(Alias::new))
                .from(Alias::new("sync_export_cursors"))
                .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
                .and_where(Expr::col(Alias::new("export_id")).eq(export_id.to_vec()))
                .to_owned(),
        ),
    )
    .one(tx)
    .await?)
}

/// Remove an export target and release its journal pin.
pub async fn remove_on(tx: &DatabaseTransaction, ns: Uuid, export_id: &[u8]) -> Result<()> {
    lock_on(tx, ns).await?;
    validate_id(export_id)?;
    let removed = tx
        .execute(
            &Query::delete()
                .from_table(Alias::new("sync_export_cursors"))
                .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
                .and_where(Expr::col(Alias::new("export_id")).eq(export_id.to_vec()))
                .to_owned(),
        )
        .await?
        .rows_affected();
    ensure!(removed == 1, "missing export cursor");
    Ok(())
}

/// Return the oldest usable export position, marking expired cursors for a base.
/// The namespace lock is held by callers of the pin calculation.
pub(crate) async fn minimum_pin_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    now_ms: i64,
) -> Result<Option<(i64, i64)>> {
    ensure!(now_ms >= 0, "invalid export cursor clock");
    let rows = tx
        .query_all(
            &Query::select()
                .columns(["export_id", "sequence", "advanced_at_ms"].map(Alias::new))
                .from(Alias::new("sync_export_cursors"))
                .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
                .and_where(Expr::col(Alias::new("needs_base")).eq(0))
                .to_owned(),
        )
        .await?;
    let (head, floor, floor_state) = position_on(tx, ns).await?;
    let mut minimum = None;
    for row in rows {
        let id: Vec<u8> = row.try_get_by_index(0)?;
        let sequence: i64 = row.try_get_by_index(1)?;
        let advanced_at_ms: i64 = row.try_get_by_index(2)?;
        ensure!(
            sequence >= 0 && advanced_at_ms >= 0,
            "invalid stored export cursor"
        );
        let rewound = now_ms < advanced_at_ms;
        let age_expired =
            !rewound && now_ms.saturating_sub(advanced_at_ms) > EXPORT_CURSOR_MAX_AGE_MS;
        let retained = tx
            .query_one(
                &Query::select()
                    .expr(Func::sum(Expr::col((
                        Alias::new("sync_manifests"),
                        Alias::new("encoded_bytes"),
                    ))))
                    .from(Alias::new("sync_journal"))
                    .join_as(
                        JoinType::InnerJoin,
                        Alias::new("sync_manifests"),
                        Alias::new("sync_manifests"),
                        Expr::col((Alias::new("sync_journal"), Alias::new("event_manifest")))
                            .equals((Alias::new("sync_manifests"), Alias::new("digest"))),
                    )
                    .and_where(
                        Expr::col((Alias::new("sync_journal"), Alias::new("namespace")))
                            .eq(namespace(ns)),
                    )
                    .and_where(
                        Expr::col((Alias::new("sync_journal"), Alias::new("sequence")))
                            .gt(sequence),
                    )
                    .to_owned(),
            )
            .await?
            .map(|row| row.try_get_by_index::<Option<i64>>(0))
            .transpose()?
            .flatten()
            .unwrap_or(0);
        let size_expired = retained > EXPORT_CURSOR_MAX_RETAINED_BYTES;
        if age_expired || size_expired || sequence < floor || sequence > head {
            tx.execute(
                &Query::update()
                    .table(Alias::new("sync_export_cursors"))
                    .value(Alias::new("needs_base"), 1_i64)
                    .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
                    .and_where(Expr::col(Alias::new("export_id")).eq(id))
                    .to_owned(),
            )
            .await?;
            continue;
        }
        if rewound {
            tx.execute(
                &Query::update()
                    .table(Alias::new("sync_export_cursors"))
                    .value(Alias::new("advanced_at_ms"), now_ms)
                    .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
                    .and_where(Expr::col(Alias::new("export_id")).eq(id))
                    .to_owned(),
            )
            .await?;
        }
        let state = if sequence == floor {
            floor_state
        } else if sequence == 0 {
            0
        } else {
            tx.query_one(
                &Query::select()
                    .column(Alias::new("state_seq"))
                    .from(Alias::new("sync_journal"))
                    .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
                    .and_where(Expr::col(Alias::new("sequence")).eq(sequence))
                    .to_owned(),
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing export cursor journal position"))?
            .try_get_by_index(0)?
        };
        let pin = (state, sequence);
        minimum = Some(minimum.map_or(pin, |current: (i64, i64)| {
            (
                std::cmp::min(current.0, pin.0),
                std::cmp::min(current.1, pin.1),
            )
        }));
    }
    Ok(minimum)
}
