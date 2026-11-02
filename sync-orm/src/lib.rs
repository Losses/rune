//! Generic SeaORM persistence for caller-defined synchronization models.
pub use causal_store::CausalReplica as OrmReplica;

pub mod causal_candidates;
pub mod causal_capabilities;
pub mod causal_compaction;
pub mod causal_gc;
pub mod causal_leases;
pub mod causal_manifests;
pub mod causal_observed;
pub mod causal_progress;
pub mod causal_schema;
pub mod causal_snapshot;
pub mod causal_source;
pub mod causal_store;
pub mod causal_writer;
pub mod maintenance;

pub mod registry;
pub mod row_policy;

pub mod derived_effect;
pub mod owned_list;
