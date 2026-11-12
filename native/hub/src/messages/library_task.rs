use rinf::{RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LibraryTaskKind {
    Scan,
    Analyze,
    Deduplicate,
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScanTaskPhase {
    IndexFiles,
    ScanCoverArts,
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
    ScanFailed,
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
        scan_phase: Option<ScanTaskPhase>,
    },
    Ended {
        record_id: String,
        task: LibraryTaskKind,
        run_id: String,
        progress: i32,
        total: i32,
        scan_phase: Option<ScanTaskPhase>,
        outcome: LibraryTaskOutcome,
    },
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct LibraryTaskStateChanged {
    pub state: LibraryTaskState,
}
