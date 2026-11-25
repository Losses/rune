//! The chart, the analysis's output: plain data in the shape of the Python prototype's charts,
//! so that it can be written as JSON and read by the Python tools (and read back, as the
//! visualizer's tests do).

use serde::{Deserialize, Serialize};

/// Names the Python tools know the analyzer by.
pub const ANALYZER: &str = "rune-1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Chart {
    pub version: u32,
    pub title: String,
    pub audio: String,
    pub duration: f64,
    pub analysis_seconds: f64,
    /// Which analyzer made the chart (an extra field; the Python's charts have none).
    pub analyzer: String,
    pub tempo: Tempo,
    pub beats: Vec<Beat>,
    pub meter: Meter,
    pub sections: Vec<Section>,
    pub streams: Streams,
    pub lead: Vec<LeadSpan>,
    pub curves: Curves,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tempo {
    /// 0 when the music has no clear pulse.
    pub bpm: f64,
    pub confidence: f64,
    /// How fast the whole song sounds, Hz.
    pub feel: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Beat {
    pub t: f64,
    pub bar: i64,
    /// 0 is the downbeat of bar `bar`.
    pub pos: i64,
    pub strength: f64,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Meter {
    pub beats_per_bar: u32,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Section {
    pub start: f64,
    pub end: f64,
    pub label: String,
    pub energy: f64,
    pub repeat: u32,
    pub confidence: f64,
}

/// One onset stream: a name for the viewer and its events, `[t, strength]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Stream {
    pub name: String,
    pub events: Vec<[f64; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Streams {
    pub low: Stream,
    pub mid: Stream,
    pub high: Stream,
    pub harmonic: Stream,
    pub phrase: Stream,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LeadSpan {
    pub start: f64,
    pub end: f64,
    /// A key of `streams`, or none when nothing stands out.
    pub stream: Option<String>,
}

/// Curves sampled every `hop` seconds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Curves {
    pub hop: f64,
    pub loudness: Vec<f64>,
    pub brightness: Vec<f64>,
    pub percussive: Vec<f64>,
    pub beat_conf: Vec<f64>,
    pub feel: Vec<f64>,
}

impl Chart {
    /// A chart with no beats and nothing else found, of a `duration` (s) long song.
    pub fn empty(duration: f64) -> Chart {
        let stream = |name: &str| Stream {
            name: name.to_string(),
            events: vec![],
        };
        Chart {
            version: 1,
            title: String::new(),
            audio: String::new(),
            duration,
            analysis_seconds: 0.0,
            analyzer: ANALYZER.to_string(),
            tempo: Tempo {
                bpm: 0.0,
                confidence: 0.0,
                feel: 0.0,
            },
            beats: vec![],
            meter: Meter {
                beats_per_bar: 4,
                confidence: 0.0,
            },
            sections: vec![],
            streams: Streams {
                low: stream("low hits"),
                mid: stream("mid hits"),
                high: stream("high hits"),
                harmonic: stream("harmonic and melodic onsets"),
                phrase: stream("phrase starts"),
            },
            lead: vec![],
            curves: Curves {
                hop: crate::np::round_to(crate::front::CHOP as f64 / crate::front::SR as f64, 6),
                loudness: vec![],
                brightness: vec![],
                percussive: vec![],
                beat_conf: vec![],
                feel: vec![],
            },
        }
    }
}
