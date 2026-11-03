//! Rune source-row policies and database replica composition.
//! Generic persistence and policy execution are implemented by `sync-orm`.
pub mod accepted;
pub mod bindings;
pub mod import;
mod legacy;
pub mod model;
pub mod references;
pub mod replica;
pub mod store;
pub use replica::LibrarySyncStore;
pub use store::OrmReplica;
pub use sync_orm::maintenance::Maintenance;
pub type SeaOrmSyncStore = OrmReplica<model::RuneModel>;

pub fn session_limits() -> sync::SessionLimits {
    sync::SessionLimits {
        pages: sync::PageLimits {
            max_scanned: 64,
            max_bytes: sync::DEFAULT_PAGE_BYTES,
        },
        stages: sync::StageLimits {
            max_total_bytes: sync::DEFAULT_STAGE_BYTES,
            max_stages: sync::DEFAULT_STAGES,
            max_event_bytes: sync::DEFAULT_LOGICAL_BYTES,
            max_event_items: sync::DEFAULT_LOGICAL_ITEMS,
            max_fragment_bytes: sync::DEFAULT_PAGE_BYTES,
        },
        apply: sync::ApplyLimits {
            max_projection_items: 64,
            max_projection_bytes: sync::DEFAULT_PROJECTION_BYTES,
            max_mutations: 65536,
        },
        work: sync::WorkLimits {
            max_requests: 128,
            max_bytes: 16 * 1024 * 1024,
            max_depth: sync::MAX_DEPTH,
            max_rows_scanned: 4096,
            max_mutations: 65536,
        },
    }
}
