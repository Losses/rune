use rinf::{RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LibraryTaskKind {
    Scan,
    Analyze,
    Deduplicate,
}

impl std::fmt::Display for LibraryTaskKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Scan => "scan",
            Self::Analyze => "analysis",
            Self::Deduplicate => "deduplication",
        })
    }
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum LibraryTaskError {
    AnalysisFailed,
    RecommendationSyncFailed,
    FingerprintingFailed,
    ComparisonFailed,
    MarkingDuplicatesFailed,
    WorkerFailed,
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum LibraryTaskOutcome {
    Finished,
    Cancelled,
    Failed(LibraryTaskError),
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum LibraryTaskState {
    NotRunning {
        record_id: String,
        task: LibraryTaskKind,
    },
    Running {
        record_id: String,
        task: LibraryTaskKind,
        run_id: String,
        progress: i32,
        total: i32,
    },
    Ended {
        record_id: String,
        task: LibraryTaskKind,
        run_id: String,
        progress: i32,
        total: i32,
        outcome: LibraryTaskOutcome,
    },
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct LibraryTaskStateChanged {
    pub state: LibraryTaskState,
}
