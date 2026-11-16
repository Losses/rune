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

/// Why a scan ended without finishing.
#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanFailure {
    ScanFailed,
    WorkerFailed,
}

/// Why an analysis ended without finishing.
#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisFailure {
    AnalysisFailed,
    RecommendationSyncFailed,
    WorkerFailed,
}

/// Why a deduplication ended without finishing.
#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeduplicationFailure {
    FingerprintingFailed,
    ComparisonFailed,
    MarkingDuplicatesFailed,
    WorkerFailed,
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanOutcome {
    Finished,
    Cancelled,
    Failed(ScanFailure),
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisOutcome {
    Finished,
    Cancelled,
    Failed(AnalysisFailure),
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeduplicationOutcome {
    Finished,
    Cancelled,
    Failed(DeduplicationFailure),
}

/// How a task run ended, for the kind of task it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryTaskEnd {
    Scan(ScanOutcome),
    Analysis(AnalysisOutcome),
    Deduplication(DeduplicationOutcome),
}

impl LibraryTaskEnd {
    pub fn kind(self) -> LibraryTaskKind {
        match self {
            Self::Scan(_) => LibraryTaskKind::Scan,
            Self::Analysis(_) => LibraryTaskKind::Analyze,
            Self::Deduplication(_) => LibraryTaskKind::Deduplicate,
        }
    }
}

/// The state of one library task. Each kind of task has its own variants, which carry
/// only the phase and the failures that kind can have.
#[derive(Deserialize, Serialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum LibraryTaskState {
    NotRunning {
        record_id: String,
        task: LibraryTaskKind,
    },
    ScanRunning {
        record_id: String,
        run_id: String,
        progress: i32,
        total: i32,
        phase: ScanTaskPhase,
    },
    ScanEnded {
        record_id: String,
        run_id: String,
        progress: i32,
        total: i32,
        phase: ScanTaskPhase,
        outcome: ScanOutcome,
    },
    AnalysisRunning {
        record_id: String,
        run_id: String,
        progress: i32,
        total: i32,
    },
    AnalysisEnded {
        record_id: String,
        run_id: String,
        progress: i32,
        total: i32,
        outcome: AnalysisOutcome,
    },
    DeduplicationRunning {
        record_id: String,
        run_id: String,
        progress: i32,
        total: i32,
    },
    DeduplicationEnded {
        record_id: String,
        run_id: String,
        progress: i32,
        total: i32,
        outcome: DeduplicationOutcome,
    },
}

impl LibraryTaskState {
    /// The run this state belongs to, for a task that has started.
    pub fn run_id(&self) -> Option<&str> {
        match self {
            Self::NotRunning { .. } => None,
            Self::ScanRunning { run_id, .. }
            | Self::ScanEnded { run_id, .. }
            | Self::AnalysisRunning { run_id, .. }
            | Self::AnalysisEnded { run_id, .. }
            | Self::DeduplicationRunning { run_id, .. }
            | Self::DeduplicationEnded { run_id, .. } => Some(run_id),
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(
            self,
            Self::ScanRunning { .. }
                | Self::AnalysisRunning { .. }
                | Self::DeduplicationRunning { .. }
        )
    }

    /// A running task ended the given way, keeping the progress it had reached unless
    /// `progress` says otherwise. Other states are returned unchanged.
    pub fn ended(self, end: LibraryTaskEnd, progress: Option<(i32, i32)>) -> Self {
        let pick = |now: (i32, i32)| progress.unwrap_or(now);
        match (self, end) {
            (
                Self::ScanRunning {
                    record_id,
                    run_id,
                    progress: now,
                    total,
                    phase,
                },
                LibraryTaskEnd::Scan(outcome),
            ) => {
                let (progress, total) = pick((now, total));
                Self::ScanEnded {
                    record_id,
                    run_id,
                    progress,
                    total,
                    phase,
                    outcome,
                }
            }
            (
                Self::AnalysisRunning {
                    record_id,
                    run_id,
                    progress: now,
                    total,
                },
                LibraryTaskEnd::Analysis(outcome),
            ) => {
                let (progress, total) = pick((now, total));
                Self::AnalysisEnded {
                    record_id,
                    run_id,
                    progress,
                    total,
                    outcome,
                }
            }
            (
                Self::DeduplicationRunning {
                    record_id,
                    run_id,
                    progress: now,
                    total,
                },
                LibraryTaskEnd::Deduplication(outcome),
            ) => {
                let (progress, total) = pick((now, total));
                Self::DeduplicationEnded {
                    record_id,
                    run_id,
                    progress,
                    total,
                    outcome,
                }
            }
            (state, _) => state,
        }
    }

    /// A running task cancelled, with the progress it had reached.
    pub fn cancelled(self) -> Self {
        let end = match &self {
            Self::ScanRunning { .. } => LibraryTaskEnd::Scan(ScanOutcome::Cancelled),
            Self::AnalysisRunning { .. } => LibraryTaskEnd::Analysis(AnalysisOutcome::Cancelled),
            Self::DeduplicationRunning { .. } => {
                LibraryTaskEnd::Deduplication(DeduplicationOutcome::Cancelled)
            }
            _ => return self,
        };
        self.ended(end, None)
    }
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct LibraryTaskStateChanged {
    pub state: LibraryTaskState,
}
