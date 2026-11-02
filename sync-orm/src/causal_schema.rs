//! Explicit installation of the causal storage layout on a fresh database.
//!
//! Existing scalar replica tables are deliberately refused: converting their
//! history requires an explicit import and cannot manufacture causal knowledge.
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, DbErr, ExprTrait, TransactionTrait,
    sea_query::{Alias, ColumnDef, Expr, Index, Query, Table},
};

#[derive(Clone, Copy)]
enum Kind {
    Bytes,
    Text,
    Integer,
}
#[derive(Clone, Copy)]
struct Field(&'static str, Kind, bool);
const B: Kind = Kind::Bytes;
const T: Kind = Kind::Text;
const I: Kind = Kind::Integer;

type Layout = (&'static str, &'static [Field], &'static [&'static str]);
const TABLES: &[Layout] = &[
    (
        "sync_meta",
        &[
            Field("namespace", B, false),
            Field("model", T, false),
            Field("schema", T, false),
            Field("codec", T, false),
            Field("replica", B, false),
            Field("generation", B, false),
            Field("head", I, false),
            Field("head_anchor", B, false),
            Field("state_seq", I, false),
            Field("revision_counter", I, false),
            Field("history_floor", I, false),
            Field("floor_anchor", B, false),
            Field("floor_state_seq", I, false),
            Field("baseline_generation", B, false),
            Field("baseline_manifest", B, true),
            Field("retained_bytes", I, false),
            Field("initialized_at", T, true),
        ],
        &["namespace"],
    ),
    (
        "sync_candidates",
        &[
            Field("namespace", B, false),
            Field("collection", B, false),
            Field("writer", B, false),
            Field("counter", I, false),
            Field("object", B, false),
            Field("author", B, false),
            Field("revision", I, false),
            Field("body", B, false),
            Field("digest", B, false),
            Field("retention_seq", I, false),
            Field("live_from", I, true),
            Field("live_until", I, true),
        ],
        &["namespace", "collection", "writer", "counter"],
    ),
    (
        "sync_journal",
        &[
            Field("namespace", B, false),
            Field("generation", B, false),
            Field("sequence", I, false),
            Field("state_seq", I, false),
            Field("collection", B, false),
            Field("event_digest", B, false),
            Field("event_manifest", B, false),
            Field("anchor_after", B, false),
        ],
        &["namespace", "generation", "sequence"],
    ),
    (
        "sync_checkpoints",
        &[
            Field("namespace", B, false),
            Field("source_replica", B, false),
            Field("source_generation", B, false),
            Field("scope_digest", B, false),
            Field("normalized_scope", B, false),
            Field("sequence", I, false),
            Field("anchor", B, false),
            Field("completed_at", T, true),
        ],
        &[
            "namespace",
            "source_replica",
            "source_generation",
            "scope_digest",
        ],
    ),
    (
        "sync_context_ranges",
        &[
            Field("namespace", B, false),
            Field("collection", B, false),
            Field("basis_generation", B, false),
            Field("state_seq", I, false),
            Field("ordinal", I, false),
            Field("lower_bound", B, false),
            Field("upper_bound", B, false),
            Field("context_manifest", B, false),
            Field("context_digest", B, true),
        ],
        &[
            "namespace",
            "collection",
            "basis_generation",
            "state_seq",
            "ordinal",
        ],
    ),
    (
        "sync_context_intervals",
        &[
            Field("namespace", B, false),
            Field("context_manifest", B, false),
            Field("writer", B, false),
            Field("first", I, false),
            Field("last", I, false),
        ],
        &["namespace", "context_manifest", "writer", "first"],
    ),
    (
        "sync_manifests",
        &[
            Field("namespace", B, false),
            Field("digest", B, false),
            Field("kind", I, false),
            Field("content_digest", B, false),
            Field("encoded_bytes", I, false),
            Field("item_count", I, false),
            Field("fragment_count", I, false),
            Field("metadata", B, false),
        ],
        &["namespace", "digest"],
    ),
    (
        "sync_chunks",
        &[
            Field("namespace", B, false),
            Field("manifest", B, false),
            Field("ordinal", I, false),
            Field("offset", I, false),
            Field("digest", B, false),
            Field("bytes", B, false),
        ],
        &["namespace", "manifest", "ordinal"],
    ),
    (
        "sync_manifest_edges",
        &[
            Field("namespace", B, false),
            Field("parent_manifest", B, false),
            Field("child_manifest", B, false),
        ],
        &["namespace", "parent_manifest", "child_manifest"],
    ),
    (
        "sync_leases",
        &[
            Field("namespace", B, false),
            Field("id", B, false),
            Field("generation", B, false),
            Field("head", I, false),
            Field("head_anchor", B, false),
            Field("state_seq", I, false),
            Field("basis_state_seq", I, true),
            Field("basis_generation", B, true),
            Field("output_generation", B, true),
            Field("parent_lease", B, true),
            Field("scope", B, false),
            Field("expires_at_ms", I, false),
            Field("status", I, false),
            Field("retained_root", B, true),
            Field("descriptor", B, true),
            Field("build_state", B, true),
        ],
        &["namespace", "id"],
    ),
];

const INDEXES: &[(&str, &str, &[&str], bool)] = &[
    (
        "sync_lease_parent",
        "sync_leases",
        &["namespace", "parent_lease", "status"],
        false,
    ),
    (
        "sync_candidate_retired",
        "sync_candidates",
        &["namespace", "live_until"],
        false,
    ),
    (
        "sync_candidate_suppressed",
        "sync_candidates",
        &["namespace", "live_from", "retention_seq"],
        false,
    ),
    (
        "sync_candidate_current",
        "sync_candidates",
        &[
            "namespace",
            "collection",
            "live_until",
            "object",
            "writer",
            "counter",
        ],
        false,
    ),
    (
        "sync_candidate_history",
        "sync_candidates",
        &[
            "namespace",
            "collection",
            "object",
            "writer",
            "counter",
            "live_from",
            "live_until",
        ],
        false,
    ),
    (
        "sync_journal_state",
        "sync_journal",
        &["namespace", "state_seq"],
        true,
    ),
    (
        "sync_journal_floor",
        "sync_journal",
        &["namespace", "sequence"],
        false,
    ),
    (
        "sync_journal_manifest",
        "sync_journal",
        &["namespace", "event_manifest"],
        false,
    ),
    (
        "sync_context_lookup",
        "sync_context_ranges",
        &[
            "namespace",
            "collection",
            "basis_generation",
            "lower_bound",
            "state_seq",
            "upper_bound",
        ],
        false,
    ),
    (
        "sync_context_manifest",
        "sync_context_ranges",
        &["namespace", "context_manifest"],
        false,
    ),
    (
        "sync_context_basis_scan",
        "sync_context_ranges",
        &[
            "namespace",
            "basis_generation",
            "collection",
            "state_seq",
            "ordinal",
        ],
        false,
    ),
    (
        "sync_context_floor",
        "sync_context_ranges",
        &["namespace", "basis_generation", "state_seq"],
        false,
    ),
    (
        "sync_interval_writer",
        "sync_context_intervals",
        &["namespace", "writer", "last", "first", "context_manifest"],
        false,
    ),
    (
        "sync_edge_child",
        "sync_manifest_edges",
        &["namespace", "child_manifest"],
        false,
    ),
    (
        "sync_lease_expiry",
        "sync_leases",
        &["namespace", "status", "expires_at_ms"],
        false,
    ),
    (
        "sync_lease_basis",
        "sync_leases",
        &["namespace", "basis_generation", "status"],
        false,
    ),
    (
        "sync_lease_output",
        "sync_leases",
        &["namespace", "output_generation", "status"],
        false,
    ),
    (
        "sync_lease_floor",
        "sync_leases",
        &["namespace", "status", "state_seq", "head"],
        false,
    ),
    (
        "sync_lease_root",
        "sync_leases",
        &["namespace", "retained_root"],
        false,
    ),
];

/// Create the complete layout in one transaction only when no replica table exists.
/// This does not convert old history; callers must import source data separately.
pub async fn install_fresh(db: &DatabaseConnection) -> Result<(), DbErr> {
    let tx = db.begin().await?;
    install_on(&tx).await?;
    tx.commit().await
}

/// Install on the migration runner's transaction.
pub async fn install_on<C: ConnectionTrait>(db: &C) -> Result<(), DbErr> {
    if db.get_database_backend() != DbBackend::Sqlite {
        return Err(DbErr::Custom(
            "causal schema currently requires SQLite".into(),
        ));
    }
    let existing = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sqlite_master"))
        .and_where(Expr::col(Alias::new("type")).eq("table"))
        .and_where(
            Expr::col(Alias::new("name")).is_in(
                TABLES
                    .iter()
                    .map(|(name, _, _)| *name)
                    .chain(["sync_objects"]),
            ),
        )
        .limit(1)
        .to_owned();
    if db.query_one(&existing).await?.is_some() {
        return Err(DbErr::Custom(
            "existing replica tables require explicit import or a fresh database".into(),
        ));
    }
    for (name, fields, keys) in TABLES {
        let mut statement = Table::create();
        statement.table(Alias::new(*name));
        for Field(field, kind, nullable) in *fields {
            let mut column = ColumnDef::new(Alias::new(*field));
            match kind {
                Kind::Bytes => {
                    column.binary();
                }
                Kind::Text => {
                    column.text();
                }
                Kind::Integer => {
                    column.big_integer();
                }
            }
            if !nullable {
                column.not_null();
            }
            if (*name == "sync_meta" && *field == "baseline_generation")
                || (*name == "sync_context_ranges" && *field == "basis_generation")
            {
                column.default(vec![0_u8; 16]);
            }
            statement.col(column);
        }
        let mut pk = Index::create();
        for key in *keys {
            pk.col(Alias::new(*key));
        }
        statement.primary_key(&mut pk);
        db.execute(&statement).await?;
    }
    for (name, table, columns, unique) in INDEXES {
        let mut index = Index::create();
        index.name(*name).table(Alias::new(*table));
        for column in *columns {
            index.col(Alias::new(*column));
        }
        if *unique {
            index.unique();
        }
        db.execute(&index).await?;
    }
    Ok(())
}

/// Remove the registered layout on the migration runner's transaction.
pub async fn remove_on<C: ConnectionTrait>(db: &C) -> Result<(), DbErr> {
    for (name, _, _) in TABLES.iter().rev() {
        db.execute(
            &Table::drop()
                .table(Alias::new(*name))
                .if_exists()
                .to_owned(),
        )
        .await?;
    }
    Ok(())
}
