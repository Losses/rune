//! Indexed temporal candidate reads for a pinned internal state position.
//! Callers must validate candidate encodings before admitting them to storage.
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, DbErr, ExprTrait, FromQueryResult,
    sea_query::{Alias, Condition, Expr, Func, Order, Query},
};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cursor {
    pub object: Uuid,
    pub writer: Uuid,
    pub counter: i64,
}
#[derive(Clone, Debug, FromQueryResult)]
pub struct CandidateRow {
    pub object: Vec<u8>,
    pub writer: Vec<u8>,
    pub counter: i64,
    pub author: Vec<u8>,
    pub revision: i64,
    pub body: Vec<u8>,
    pub digest: Vec<u8>,
    pub live_from: Option<i64>,
    pub live_until: Option<i64>,
}
#[derive(Debug)]
pub struct CandidatePage {
    pub rows: Vec<CandidateRow>,
    pub next: Option<Cursor>,
    pub done: bool,
}
fn invalid(message: &str) -> DbErr {
    DbErr::Custom(message.into())
}
fn cursor(row: &CandidateRow) -> Result<Cursor, DbErr> {
    if row.counter <= 0
        || row.revision <= 0
        || row.digest.len() != 32
        || row.author.len() != 16
        || row.body.len() > 64 * 1024
    {
        return Err(invalid("invalid stored candidate"));
    }
    let object = Uuid::from_slice(&row.object).map_err(|_| invalid("invalid object UUID"))?;
    let writer = Uuid::from_slice(&row.writer).map_err(|_| invalid("invalid writer UUID"))?;
    let author = Uuid::from_slice(&row.author).map_err(|_| invalid("invalid author UUID"))?;
    if object.is_nil()
        || writer.is_nil()
        || author.is_nil()
        || row.live_from.is_none_or(|v| v <= 0)
        || row
            .live_until
            .zip(row.live_from)
            .is_some_and(|(end, start)| end <= start)
    {
        return Err(invalid("invalid stored candidate"));
    }
    Ok(Cursor {
        object,
        writer,
        counter: row.counter,
    })
}
/// Read an ordered page without retaining a database transaction across calls.
/// `after` is exclusive; upper bound is exclusive. A caller must hold a valid pin
/// for `state_seq` and enforce the same candidate size limit on ingestion.
/// The byte limit bounds this helper's body buffer; the caller must separately
/// enforce the exact encoded transport envelope size.
pub struct CandidatePageRequest<'a> {
    pub namespace: Uuid,
    pub collection: &'a [u8],
    pub state_seq: i64,
    pub lower: Option<Uuid>,
    pub upper: Option<Uuid>,
    pub after: Option<Cursor>,
    pub max_items: usize,
    pub max_bytes: usize,
}

pub async fn page_on(
    tx: &DatabaseTransaction,
    request: CandidatePageRequest<'_>,
) -> Result<CandidatePage, DbErr> {
    let CandidatePageRequest {
        namespace: ns,
        collection,
        state_seq,
        lower,
        upper,
        after,
        max_items,
        max_bytes,
    } = request;
    const MAX_ITEMS: usize = 1024;
    const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
    if ns.is_nil()
        || collection.is_empty()
        || collection.len() > 128
        || state_seq < 0
        || max_items == 0
        || max_items > MAX_ITEMS
        || max_bytes == 0
        || max_bytes > MAX_PAGE_BYTES
        || lower.zip(upper).is_some_and(|(l, u)| l >= u)
    {
        return Err(invalid("invalid candidate page request"));
    }
    if let Some(ref c) = after
        && (c.counter <= 0
            || c.object.is_nil()
            || c.writer.is_nil()
            || lower.is_some_and(|x| c.object < x)
            || upper.is_some_and(|x| c.object >= x))
    {
        return Err(invalid("invalid candidate cursor"));
    }
    let mut rows = Vec::new();
    let mut next = after;
    let mut bytes = 0usize;
    loop {
        // One extra header probes for more while keeping parameter counts bounded.
        let fetch = std::cmp::min(
            max_items.saturating_sub(rows.len()).saturating_add(1),
            MAX_HEADER_BATCH,
        );
        let headers = header_batch_on(
            tx,
            CandidatePageRequest {
                namespace: ns,
                collection,
                state_seq,
                lower,
                upper,
                after: next.clone(),
                max_items: fetch,
                max_bytes,
            },
        )
        .await?;
        if headers.is_empty() {
            return Ok(CandidatePage {
                rows,
                next,
                done: true,
            });
        }
        let mut admitted = 0;
        let mut admitted_bytes = 0usize;
        for header in &headers {
            let body_len = usize::try_from(header.body_len)
                .map_err(|_| invalid("invalid stored candidate size"))?;
            if body_len > 64 * 1024 {
                return Err(invalid("invalid stored candidate size"));
            }
            let size = body_len
                .checked_add(96)
                .ok_or_else(|| invalid("candidate size overflow"))?;
            if rows.len() + admitted >= max_items
                || bytes
                    .checked_add(admitted_bytes)
                    .and_then(|n| n.checked_add(size))
                    .is_none_or(|n| n > max_bytes)
            {
                if rows.is_empty() && admitted == 0 {
                    return Err(invalid("candidate does not fit requested page"));
                }
                break;
            }
            admitted += 1;
            admitted_bytes += size;
        }
        let bodies = body_batch_on(tx, ns, collection, &headers[..admitted]).await?;
        let mut body_map = std::collections::BTreeMap::new();
        for body in bodies {
            if body_map
                .insert((body.writer, body.counter), body.body)
                .is_some()
            {
                return Err(invalid("duplicate candidate body"));
            }
        }
        for header in headers.iter().take(admitted) {
            let body = body_map
                .remove(&(header.writer.clone(), header.counter))
                .ok_or_else(|| invalid("candidate disappeared or changed during page read"))?;
            if body.len()
                != usize::try_from(header.body_len)
                    .map_err(|_| invalid("invalid stored candidate size"))?
            {
                return Err(invalid("candidate changed during page read"));
            }
            let row = CandidateRow {
                object: header.object.clone(),
                writer: header.writer.clone(),
                counter: header.counter,
                author: header.author.clone(),
                revision: header.revision,
                body,
                digest: header.digest.clone(),
                live_from: header.live_from,
                live_until: header.live_until,
            };
            next = Some(cursor(&row)?);
            rows.push(row);
        }
        bytes += admitted_bytes;
        if admitted < headers.len() {
            return Ok(CandidatePage {
                rows,
                next,
                done: false,
            });
        }
        if headers.len() < fetch {
            return Ok(CandidatePage {
                rows,
                next,
                done: true,
            });
        }
    }
}

pub(crate) const MAX_HEADER_BATCH: usize = 200;

/// Header returned without loading the retained body; callers validate fields.
#[derive(FromQueryResult)]
pub(crate) struct BatchHeader {
    pub object: Vec<u8>,
    pub writer: Vec<u8>,
    pub counter: i64,
    pub author: Vec<u8>,
    pub revision: i64,
    pub digest: Vec<u8>,
    pub body_len: i64,
    pub live_from: Option<i64>,
    pub live_until: Option<i64>,
}

pub(crate) async fn header_batch_on(
    tx: &DatabaseTransaction,
    request: CandidatePageRequest<'_>,
) -> Result<Vec<BatchHeader>, DbErr> {
    let CandidatePageRequest {
        namespace,
        collection,
        state_seq,
        lower,
        upper,
        after,
        max_items,
        ..
    } = request;
    if namespace.is_nil()
        || collection.is_empty()
        || collection.len() > 128
        || state_seq < 0
        || max_items == 0
        || max_items > MAX_HEADER_BATCH
        || lower.zip(upper).is_some_and(|(a, b)| a >= b)
    {
        return Err(invalid("invalid candidate header batch"));
    }
    if after.as_ref().is_some_and(|cursor| {
        cursor.object.is_nil()
            || cursor.writer.is_nil()
            || cursor.counter <= 0
            || lower.is_some_and(|value| cursor.object < value)
            || upper.is_some_and(|value| cursor.object >= value)
    }) {
        return Err(invalid("invalid candidate header cursor"));
    }
    let mut query = Query::select();
    query
        .columns(
            [
                "object",
                "writer",
                "counter",
                "author",
                "revision",
                "digest",
                "live_from",
                "live_until",
            ]
            .map(Alias::new),
        )
        .expr_as(
            Func::cust(Alias::new("length")).arg(Expr::col(Alias::new("body"))),
            Alias::new("body_len"),
        )
        .from(Alias::new("sync_candidates"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("collection")).eq(collection.to_vec()))
        .and_where(Expr::col(Alias::new("live_from")).is_not_null())
        .and_where(Expr::col(Alias::new("live_from")).lte(state_seq))
        .cond_where(
            Condition::any()
                .add(Expr::col(Alias::new("live_until")).is_null())
                .add(Expr::col(Alias::new("live_until")).gt(state_seq)),
        );
    if let Some(lower) = lower {
        query.and_where(Expr::col(Alias::new("object")).gte(lower.as_bytes().to_vec()));
    }
    if let Some(upper) = upper {
        query.and_where(Expr::col(Alias::new("object")).lt(upper.as_bytes().to_vec()));
    }
    if let Some(c) = after {
        let object = c.object.as_bytes().to_vec();
        let writer = c.writer.as_bytes().to_vec();
        query.cond_where(
            Condition::any()
                .add(Expr::col(Alias::new("object")).gt(object.clone()))
                .add(
                    Condition::all()
                        .add(Expr::col(Alias::new("object")).eq(object.clone()))
                        .add(Expr::col(Alias::new("writer")).gt(writer.clone())),
                )
                .add(
                    Condition::all()
                        .add(Expr::col(Alias::new("object")).eq(object))
                        .add(Expr::col(Alias::new("writer")).eq(writer))
                        .add(Expr::col(Alias::new("counter")).gt(c.counter)),
                ),
        );
    }
    query
        .order_by(Alias::new("object"), Order::Asc)
        .order_by(Alias::new("writer"), Order::Asc)
        .order_by(Alias::new("counter"), Order::Asc)
        .limit(max_items as u64);
    BatchHeader::find_by_statement(tx.get_database_backend().build(&query))
        .all(tx)
        .await
}

#[derive(FromQueryResult)]
pub(crate) struct BatchBody {
    pub writer: Vec<u8>,
    pub counter: i64,
    pub body: Vec<u8>,
}

/// The length predicate is checked by the database before body allocation.
pub(crate) async fn body_batch_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    collection: &[u8],
    headers: &[BatchHeader],
) -> Result<Vec<BatchBody>, DbErr> {
    if headers.is_empty() {
        return Ok(Vec::new());
    }
    if headers.len() > MAX_HEADER_BATCH
        || headers
            .iter()
            .any(|h| h.body_len < 0 || h.body_len > 64 * 1024)
    {
        return Err(invalid("invalid candidate body batch"));
    }
    let mut alternatives = Condition::any();
    for header in headers {
        alternatives = alternatives.add(
            Condition::all()
                .add(Expr::col(Alias::new("writer")).eq(header.writer.clone()))
                .add(Expr::col(Alias::new("counter")).eq(header.counter))
                .add(
                    Expr::expr(Func::cust(Alias::new("length")).arg(Expr::col(Alias::new("body"))))
                        .eq(header.body_len),
                ),
        );
    }
    let query = Query::select()
        .columns(["writer", "counter", "body"].map(Alias::new))
        .from(Alias::new("sync_candidates"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("collection")).eq(collection.to_vec()))
        .cond_where(alternatives)
        .to_owned();
    BatchBody::find_by_statement(tx.get_database_backend().build(&query))
        .all(tx)
        .await
}
