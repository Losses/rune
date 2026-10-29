//! How long each frame takes on the GPU, for the adaptive quality ([`crate::adaptive`]).
//!
//! Where the device has timestamp queries (`Features::TIMESTAMP_QUERY`: Metal on Apple GPUs,
//! DirectX 12, Vulkan when the queue has timestamp bits) the scene's first pass writes one at its
//! beginning and the last pass (the post-processing's) one at its end. Only pass boundaries are
//! used, which is all Apple GPUs offer (no timestamps inside encoders). Once the frame is done they
//! are resolved into a small buffer that maps, so the time arrives a few frames later and nothing
//! waits for it. On a GPU busy with other work too (or with frames queued faster than it draws),
//! passes of other work can run in between, so the time is how long the frame took, not only what
//! it cost.
//!
//! Without them the time is the wall time from submitting the frame until the GPU finished it, so
//! the render thread waits for every frame. That gives up the overlap of encoding one frame with
//! drawing the ones before (see [`crate::in_flight`]), which only adapters without timestamps pay.
//! A time taken without waiting would not be one: a frame queued behind others would count their
//! time too, and the moment a frame finished is only seen when the render thread next looks.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

const FREE: u8 = 0;
/// Written into by a frame on its way.
const WRITTEN: u8 = 1;
/// The frame is done, the timestamps are ready to resolve.
const DONE: u8 = 2;
/// Resolved, mapping.
const PENDING: u8 = 3;
const MAPPED: u8 = 4;
const FAILED: u8 = 5;

/// Frames whose timestamps can be on their way at once; a frame that finds none free is not
/// timed.
const SLOTS: usize = 6;
/// Bytes of the two timestamps.
const SIZE: u64 = 2 * wgpu::QUERY_SIZE as u64;
/// Longer than this is not a frame's time but a stall, a device lost or a broken timestamp.
const LONGEST: f64 = 1.0;
/// How long the fallback waits for a frame at most.
const WAIT: Duration = Duration::from_secs(1);

/// The timestamps of one frame: a query set of its own, resolved once the frame is done (Apple
/// GPUs may not have written the end of the last pass yet when a later command of the same
/// submission resolves it), and copied into a buffer that maps.
struct Slot {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

enum Measure {
    Timestamps {
        slots: Vec<Slot>,
        /// The slot of the frame being encoded.
        current: Option<usize>,
        /// Nanoseconds per tick.
        period: f64,
    },
    /// When the frame was encoded, until it was submitted.
    Wall(Option<Instant>),
}

pub struct GpuTimer {
    measure: Measure,
    /// Frame times measured by the fallback since the last [`Self::collect`].
    done: Vec<f64>,
}

impl GpuTimer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return Self {
                measure: Measure::Wall(None),
                done: Vec::new(),
            };
        }
        let buffer = |usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("visualizer frame time"),
                size: SIZE,
                usage,
                mapped_at_creation: false,
            })
        };
        let slots = (0..SLOTS)
            .map(|_| Slot {
                set: device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("visualizer frame time"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                }),
                resolve: buffer(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC),
                buffer: buffer(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
                state: Arc::new(AtomicU8::new(FREE)),
            })
            .collect();
        Self {
            measure: Measure::Timestamps {
                slots,
                current: None,
                period: f64::from(queue.get_timestamp_period()),
            },
            done: Vec::new(),
        }
    }

    /// Whether frames are timed with timestamp queries (else by waiting for them).
    pub fn has_timestamps(&self) -> bool {
        matches!(self.measure, Measure::Timestamps { .. })
    }

    /// A frame is about to be encoded: the query set its passes write timestamps 0 (at the
    /// beginning of the first) and 1 (at the end of the last) into, if it is timed that way.
    pub fn begin(&mut self) -> Option<&wgpu::QuerySet> {
        match &mut self.measure {
            Measure::Timestamps { slots, current, .. } => {
                if current.is_none() {
                    *current = slots
                        .iter()
                        .position(|s| s.state.load(Ordering::Acquire) == FREE);
                }
                current.map(|i| &slots[i].set)
            }
            Measure::Wall(_) => None,
        }
    }

    /// The frame is encoded into `encoder`, which is submitted next.
    pub fn end(&mut self, encoder: &mut wgpu::CommandEncoder) {
        match &mut self.measure {
            Measure::Timestamps { slots, current, .. } => {
                let Some(i) = current.take() else {
                    return;
                };
                let state = &slots[i].state;
                state.store(WRITTEN, Ordering::Release);
                let state = Arc::clone(state);
                encoder.on_submitted_work_done(move || state.store(DONE, Ordering::Release));
            }
            Measure::Wall(started) => *started = Some(Instant::now()),
        }
    }

    /// The frame [`Self::end`] saw was submitted (or dropped). The fallback waits for it here.
    pub fn submitted(&mut self, device: &wgpu::Device) {
        let Measure::Wall(started) = &mut self.measure else {
            return;
        };
        let Some(started) = started.take() else {
            return;
        };
        let waited = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(WAIT),
        });
        let seconds = started.elapsed().as_secs_f64();
        if waited.is_ok() && seconds < LONGEST {
            self.done.push(seconds);
        }
    }

    /// The GPU times (seconds) of the frames whose timestamps arrived since the last call, into
    /// `times`. Resolves those of the frames that are done since.
    pub fn collect(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, times: &mut Vec<f64>) {
        times.append(&mut self.done);
        let Measure::Timestamps { slots, period, .. } = &self.measure else {
            return;
        };
        // Runs the callbacks of finished frames and mappings without blocking.
        if device.poll(wgpu::PollType::Poll).is_err() {
            return;
        }
        let mut encoder = None;
        for slot in slots {
            match slot.state.load(Ordering::Acquire) {
                DONE => {
                    let encoder = encoder
                        .get_or_insert_with(|| device.create_command_encoder(&Default::default()));
                    encoder.resolve_query_set(&slot.set, 0..2, &slot.resolve, 0);
                    encoder.copy_buffer_to_buffer(&slot.resolve, 0, &slot.buffer, 0, SIZE);
                    slot.state.store(PENDING, Ordering::Release);
                    let state = Arc::clone(&slot.state);
                    encoder.map_buffer_on_submit(&slot.buffer, wgpu::MapMode::Read, .., move |r| {
                        state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
                    });
                }
                MAPPED => {
                    if let Ok(view) = slot.buffer.slice(..).get_mapped_range() {
                        let tick = |i: usize| {
                            u64::from_le_bytes(view[i * 8..i * 8 + 8].try_into().unwrap_or([0; 8]))
                        };
                        let (begin, end) = (tick(0), tick(1));
                        let seconds = end.saturating_sub(begin) as f64 * period * 1e-9;
                        if end > begin && seconds < LONGEST {
                            times.push(seconds);
                        }
                    }
                    slot.buffer.unmap();
                    slot.state.store(FREE, Ordering::Release);
                }
                FAILED => slot.state.store(FREE, Ordering::Release),
                _ => {}
            }
        }
        if let Some(encoder) = encoder {
            queue.submit(Some(encoder.finish()));
        }
    }
}
