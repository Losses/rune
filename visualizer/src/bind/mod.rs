//! Declarative bindings: what the music drives, how, and what happens where it cannot be
//! measured (see "Bindings" in `docs/music.md`).
//!
//! A scene declares the inputs it wants (`[[input]]`) and binds each to a source
//! ([`sources`]) with a `how` ([`eval`]); the runtime works the values out every frame and the
//! scene reads them as `input.<id>`. Which fallback the music is at ([`level`]) decides, per
//! frame, whether a source is the thing it says or the binding's fallback takes over.

pub mod declared;
mod eval;
mod level;
mod runtime;
#[cfg(test)]
pub(crate) use runtime::tests;
mod sources;

pub use eval::{
    Bindings, Curve, Ease, Env, Evaluator, HOWS, MAX_OPTIONS, Report, Scope, Tick, Used,
};
pub use level::{Level, forced_chart};
pub use runtime::EMBEDDED as RUNTIME_TOML;
pub use runtime::{
    Ahead, CourseBinds, DirectorBinds, LookBinds, LookCue, MomentBinds, Pick, RuntimeBindings,
    Said, Trigger, counts_here,
};
pub use sources::{Reading, SOURCES, Source, audible, find, names};
