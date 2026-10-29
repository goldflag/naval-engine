//! GPU timings per pass, from pass-level timestamp writes.
//!
//! Every pass the renderer records (shadow maps, scene passes, post passes, the display pass)
//! takes a begin and an end timestamp when the device has `Features::TIMESTAMP_QUERY`; layers can
//! time their own passes through [`GpuTimer::render_pass`] and [`GpuTimer::compute_pass`]. Only
//! pass-level writes are used: Metal on Apple silicon advertises neither
//! `TIMESTAMP_QUERY_INSIDE_ENCODERS` nor `TIMESTAMP_QUERY_INSIDE_PASSES`.
//!
//! **A frame's queries are resolved in a later submission, once that frame's work is done.**
//! On Metal, a `resolve_query_set` in the same command buffer can run before the last pass's
//! end-of-fragment sample is written (the counter sample buffer is not hazard-tracked), which
//! reads back zero or a stale value (an end before its begin). So each frame's query set waits
//! in a ring until `on_submitted_work_done` says the frame finished, is resolved in the next
//! frame's encoder, and is mapped once that submission finishes: results arrive two or three
//! frames late, without ever blocking.
//!
//! **Spans overlap on a tile-based GPU.** A render pass's begin is the start of its vertex
//! stage, which may run ahead of the previous pass's fragment work, so a pass's span (end −
//! begin) can include waiting. Each pass also gets a serial cost, end − max(begin, the previous
//! passes' latest end), which sums to the frame's busy time; that is the number to budget with.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Passes timed per frame at most; a frame with more times the first ones.
pub const MAX_TIMED_PASSES: u32 = 48;
/// Frames of timings kept.
const HISTORY: usize = 240;
/// Query sets in flight: one recording, one or two waiting for their frame, one resolving.
const SLOTS: usize = 4;

/// One pass's time.
#[derive(Clone, Debug, PartialEq)]
pub struct PassTiming {
    pub label: String,
    /// End − begin, milliseconds. `None` when a sample read back zero or out of order.
    pub span_ms: Option<f64>,
    /// End − max(begin, the latest end before it), milliseconds: this pass's share of the frame.
    pub serial_ms: Option<f64>,
}

/// One frame's GPU timings.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameTimings {
    pub frame: u64,
    pub passes: Vec<PassTiming>,
    /// The latest end − the earliest begin, milliseconds.
    pub total_ms: Option<f64>,
}

impl FrameTimings {
    /// Turns raw begin/end ticks (pairs per label, in recording order) into timings.
    pub fn from_ticks(frame: u64, labels: &[String], ticks: &[u64], period_ns: f32) -> Self {
        let mut timings = Self {
            frame,
            passes: Vec::with_capacity(labels.len()),
            total_ms: None,
        };
        timings.set_from_ticks(frame, labels, ticks, period_ns);
        timings
    }

    /// [`FrameTimings::from_ticks`] written over these timings: their list and labels keep their room.
    pub fn set_from_ticks(&mut self, frame: u64, labels: &[String], ticks: &[u64], period_ns: f32) {
        let ms = |ticks: u64| ticks as f64 * f64::from(period_ns) / 1e6;
        let passes = &mut self.passes;
        passes.truncate(labels.len());
        let mut latest_end: Option<u64> = None;
        let mut first_begin: Option<u64> = None;
        for (index, label) in labels.iter().enumerate() {
            let (begin, end) = (ticks[2 * index], ticks[2 * index + 1]);
            let valid = begin != 0 && end != 0 && end >= begin;
            let (span_ms, serial_ms) = if valid {
                let start = latest_end.map_or(begin, |latest| latest.max(begin));
                first_begin = Some(first_begin.map_or(begin, |first| first.min(begin)));
                latest_end = Some(latest_end.map_or(end, |latest| latest.max(end)));
                (Some(ms(end - begin)), Some(ms(end.saturating_sub(start))))
            } else {
                (None, None)
            };
            match passes.get_mut(index) {
                Some(pass) => {
                    pass.label.clone_from(label);
                    pass.span_ms = span_ms;
                    pass.serial_ms = serial_ms;
                }
                None => passes.push(PassTiming {
                    label: label.clone(),
                    span_ms,
                    serial_ms,
                }),
            }
        }
        self.frame = frame;
        self.total_ms = match (first_begin, latest_end) {
            (Some(first), Some(last)) => Some(ms(last - first)),
            _ => None,
        };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotState {
    Free,
    /// Taking this frame's timestamps.
    Recording,
    /// Submitted; waiting for the frame to finish.
    Submitted,
    /// Resolved in a later frame's encoder; waiting for that submission to finish.
    Resolving,
    /// Mapping the read-back.
    Mapping,
}

struct Slot {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    state: SlotState,
    frame: u64,
    labels: Vec<String>,
    /// Set when the submission this slot waits on has finished.
    done: Arc<AtomicBool>,
    /// Set when the read-back is mapped (or mapping failed).
    mapped: Arc<Mutex<Option<bool>>>,
}

/// Timestamp queries for the renderer's passes and any layer's.
pub struct GpuTimer {
    slots: Vec<Slot>,
    /// The slot taking this frame's timestamps, if any is free and timing is on.
    recording: Option<usize>,
    /// Labels of this frame's timed passes so far: the first `label_count` (interior mutability: passes are
    /// opened while the rest of the renderer is borrowed). The rest are room kept from earlier frames.
    labels: RefCell<Vec<String>>,
    label_count: Cell<usize>,
    /// Read-back ticks, finished frames, and timings dropped from the history, kept for reuse (no allocation per
    /// frame once the history is full).
    ticks: Vec<u64>,
    finished: Vec<FrameTimings>,
    spare: Vec<FrameTimings>,
    /// Flags to set when this frame's submission finishes.
    pending: Vec<Arc<AtomicBool>>,
    period_ns: f32,
    history: VecDeque<FrameTimings>,
    enabled: bool,
}

impl GpuTimer {
    /// A timer on `device`; timing is off without `Features::TIMESTAMP_QUERY`.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let supported = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let slots = if supported {
            (0..SLOTS)
                .map(|index| {
                    let bytes = u64::from(MAX_TIMED_PASSES) * 2 * 8;
                    Slot {
                        queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                            label: Some(&format!("gpu timer {index}")),
                            ty: wgpu::QueryType::Timestamp,
                            count: MAX_TIMED_PASSES * 2,
                        }),
                        resolve: device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("gpu timer resolve"),
                            size: bytes,
                            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                            mapped_at_creation: false,
                        }),
                        readback: device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("gpu timer read-back"),
                            size: bytes,
                            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                            mapped_at_creation: false,
                        }),
                        state: SlotState::Free,
                        frame: 0,
                        labels: Vec::new(),
                        done: Arc::new(AtomicBool::new(false)),
                        mapped: Arc::new(Mutex::new(None)),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            slots,
            recording: None,
            labels: RefCell::new(Vec::new()),
            label_count: Cell::new(0),
            ticks: Vec::new(),
            finished: Vec::new(),
            spare: Vec::new(),
            pending: Vec::new(),
            period_ns: queue.get_timestamp_period(),
            history: VecDeque::new(),
            enabled: supported,
        }
    }

    /// Whether the device can take timestamps.
    pub fn supported(&self) -> bool {
        !self.slots.is_empty()
    }

    /// Whether passes are timed (supported and not switched off).
    pub fn enabled(&self) -> bool {
        self.enabled && self.supported()
    }

    /// Switches timing on or off (on by default where supported).
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Advances the ring: resolves finished frames into `encoder`, maps finished resolves, reads
    /// mapped results, and takes a free slot for this frame.
    pub(crate) fn begin_frame(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        frame: u64,
    ) {
        self.advance(device, encoder);
        self.label_count.set(0);
        self.recording = None;
        if !self.enabled() {
            return;
        }
        if let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.state == SlotState::Free)
        {
            let slot = &mut self.slots[index];
            slot.state = SlotState::Recording;
            slot.frame = frame;
            slot.done.store(false, Ordering::SeqCst);
            self.pending.push(slot.done.clone());
            self.recording = Some(index);
        }
    }

    /// Moves every slot along as far as it can go without waiting.
    fn advance(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        if self.slots.is_empty() {
            return;
        }
        // Native wgpu runs callbacks when polled; on the web they run on their own.
        let _ = device.poll(wgpu::PollType::Poll);
        let finished = &mut self.finished;
        finished.clear();
        for slot in &mut self.slots {
            match slot.state {
                SlotState::Submitted if slot.done.load(Ordering::SeqCst) => {
                    let count = slot.labels.len() as u32 * 2;
                    if count == 0 {
                        slot.state = SlotState::Free;
                        continue;
                    }
                    encoder.resolve_query_set(&slot.queries, 0..count, &slot.resolve, 0);
                    encoder.copy_buffer_to_buffer(
                        &slot.resolve,
                        0,
                        &slot.readback,
                        0,
                        u64::from(count) * 8,
                    );
                    slot.state = SlotState::Resolving;
                    slot.done.store(false, Ordering::SeqCst);
                    self.pending.push(slot.done.clone());
                }
                SlotState::Resolving if slot.done.load(Ordering::SeqCst) => {
                    *slot.mapped.lock().expect("not poisoned") = None;
                    let mapped = slot.mapped.clone();
                    let bytes = slot.labels.len() as u64 * 16;
                    slot.readback
                        .slice(..bytes)
                        .map_async(wgpu::MapMode::Read, move |result| {
                            *mapped.lock().expect("not poisoned") = Some(result.is_ok());
                        });
                    slot.state = SlotState::Mapping;
                }
                SlotState::Mapping => {
                    let mapped = *slot.mapped.lock().expect("not poisoned");
                    match mapped {
                        Some(true) => {
                            let bytes = slot.labels.len() as u64 * 16;
                            self.ticks.clear();
                            {
                                let view = slot
                                    .readback
                                    .slice(..bytes)
                                    .get_mapped_range()
                                    .expect("mapped");
                                self.ticks.extend(
                                    view.as_chunks::<8>()
                                        .0
                                        .iter()
                                        .map(|bytes| u64::from_le_bytes(*bytes)),
                                );
                            }
                            slot.readback.unmap();
                            slot.state = SlotState::Free;
                            // Written over timings the history let go of, where there are any.
                            let timings = match self.spare.pop() {
                                Some(mut timings) => {
                                    timings.set_from_ticks(
                                        slot.frame,
                                        &slot.labels,
                                        &self.ticks,
                                        self.period_ns,
                                    );
                                    timings
                                }
                                None => FrameTimings::from_ticks(
                                    slot.frame,
                                    &slot.labels,
                                    &self.ticks,
                                    self.period_ns,
                                ),
                            };
                            finished.push(timings);
                        }
                        Some(false) => slot.state = SlotState::Free,
                        None => {}
                    }
                }
                _ => {}
            }
        }
        finished.sort_by_key(|timings| timings.frame);
        for timings in finished.drain(..) {
            self.history.push_back(timings);
            while self.history.len() > HISTORY {
                if let Some(old) = self.history.pop_front() {
                    self.spare.push(old);
                }
            }
        }
    }

    /// Hands the flags of this frame's submission to `queue`: each is set when it finishes.
    pub(crate) fn end_frame(&mut self, queue: &wgpu::Queue) {
        if let Some(index) = self.recording.take() {
            let slot = &mut self.slots[index];
            // Copied over the slot's own labels, which keep their room.
            let labels = self.labels.borrow();
            let labels = &labels[..self.label_count.get()];
            slot.labels.truncate(labels.len());
            for (i, label) in labels.iter().enumerate() {
                match slot.labels.get_mut(i) {
                    Some(held) => held.clone_from(label),
                    None => slot.labels.push(label.clone()),
                }
            }
            slot.state = SlotState::Submitted;
        }
        if self.pending.is_empty() {
            return;
        }
        let flags = std::mem::take(&mut self.pending);
        queue.on_submitted_work_done(move || {
            for flag in flags {
                flag.store(true, Ordering::SeqCst);
            }
        });
    }

    /// Resolves and reads what finished frames it can without starting a frame: `encoder` is
    /// submitted by the caller. The harness calls this after waiting for the GPU, to drain the
    /// ring.
    pub fn drain(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpu timer drain"),
        });
        self.advance(device, &mut encoder);
        queue.submit([encoder.finish()]);
        if !self.pending.is_empty() {
            let flags = std::mem::take(&mut self.pending);
            queue.on_submitted_work_done(move || {
                for flag in flags {
                    flag.store(true, Ordering::SeqCst);
                }
            });
        }
    }

    fn next_pair(&self, label: &str) -> Option<(&wgpu::QuerySet, u32)> {
        let slot = &self.slots[self.recording?];
        let count = self.label_count.get();
        if count as u32 >= MAX_TIMED_PASSES {
            return None;
        }
        // Written over a label kept from an earlier frame where there is one.
        let mut labels = self.labels.borrow_mut();
        match labels.get_mut(count) {
            Some(held) => {
                held.clear();
                held.push_str(label);
            }
            None => labels.push(label.to_owned()),
        }
        self.label_count.set(count + 1);
        Some((&slot.queries, count as u32 * 2))
    }

    /// Timestamp writes for a render pass labelled `label`, or `None` when timing is off or
    /// the frame's queries are used up. Put it in the pass descriptor's `timestamp_writes`.
    pub fn render_pass(&self, label: &str) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let (query_set, first) = self.next_pair(label)?;
        Some(wgpu::RenderPassTimestampWrites {
            query_set,
            beginning_of_pass_write_index: Some(first),
            end_of_pass_write_index: Some(first + 1),
        })
    }

    /// Timestamp writes for a compute pass labelled `label`.
    pub fn compute_pass(&self, label: &str) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        let (query_set, first) = self.next_pair(label)?;
        Some(wgpu::ComputePassTimestampWrites {
            query_set,
            beginning_of_pass_write_index: Some(first),
            end_of_pass_write_index: Some(first + 1),
        })
    }

    /// The latest frame whose timings have arrived.
    pub fn latest(&self) -> Option<&FrameTimings> {
        self.history.back()
    }

    /// Timings of recent frames, oldest first.
    pub fn history(&self) -> impl Iterator<Item = &FrameTimings> {
        self.history.iter()
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Nanoseconds per timestamp tick.
    pub fn period_ns(&self) -> f32 {
        self.period_ns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_costs_do_not_count_overlap_twice() {
        let labels: Vec<String> = ["a", "b", "c", "bad"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // a: 0..10 ms; b begins at 4 ms (its vertex stage ran ahead) and ends at 16; c is a
        // clean 16..20; `bad` read back zero.
        let ms = 1_000_000;
        let ticks = [1, 10 * ms, 4 * ms, 16 * ms, 16 * ms, 20 * ms, 0, 0];
        let timings = FrameTimings::from_ticks(7, &labels, &ticks, 1.0);
        let spans: Vec<_> = timings.passes.iter().map(|p| p.span_ms).collect();
        let serial: Vec<_> = timings.passes.iter().map(|p| p.serial_ms).collect();
        let round = |v: Option<f64>| v.map(|v| (v * 1000.0).round() / 1000.0);
        assert_eq!(
            spans.into_iter().map(round).collect::<Vec<_>>(),
            [Some(10.0), Some(12.0), Some(4.0), None]
        );
        assert_eq!(
            serial.into_iter().map(round).collect::<Vec<_>>(),
            [Some(10.0), Some(6.0), Some(4.0), None]
        );
        assert_eq!(round(timings.total_ms), Some(20.0));
        // An end before its begin is a stale sample, not a negative time.
        let timings = FrameTimings::from_ticks(8, &labels[..1], &[5 * ms, 2 * ms], 1.0);
        assert_eq!(timings.passes[0].span_ms, None);
        assert_eq!(timings.total_ms, None);
    }
}
