//! The land's own shadow as the sun moves (native/docs/terrain/shadows.md, "The land's own shadow"): the field's line
//! sweep
//! (`PreparedField::sunlight`) for the light, uploaded to the texture the land samples. A theatre window's sweep is 5.8
//! million samples, 25–28 ms of CPU, so once the sun has moved past [`SUN_TOLERANCE`] a host that hands the layer a
//! [`LandWorker`] has it computed on one of its threads, the old shadow drawn until the new one is done; the first
//! `prepare` after that uploads it (5.8 MB, about a millisecond). The upload stays on the render thread: wgpu holds its
//! queue's pending-writes lock through a texture write's copy, and a worker below the frame loop's class holding it
//! stalled the frame loop's own writes behind it (18–20 ms in a scene's first frame, measured). Without a worker, or with
//! the layer's background work off (a manual clock's scripted frames, which must repeat), the render thread computes it
//! as before, and so it does a land's first shadow when it was made without one (better a frame's cost than land lit
//! everywhere for a few). One job at a time; a light that moved on meanwhile is taken after it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use glam::Vec3;

use crate::field::PreparedField;
use crate::textures::write_levels;

/// Sun movement (radians) that recomputes the land's shadow: about 0.3°.
pub const SUN_TOLERANCE: f32 = 0.005;

/// Work the land hands its host to run on another thread (the host's scheduling class, its thread or pool). A host
/// that cannot start a thread drops the job: the land then does the work itself.
pub type LandWorker = Arc<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;

/// A shadow being computed on the worker: its light, and the computed bytes once done.
struct SunJob {
    direction: Vec3,
    finished: AtomicBool,
    /// The buffer it computed into, once done; none if the job was dropped unrun or panicked.
    bytes: Mutex<Option<Vec<u8>>>,
}

/// Marks the job finished however the worker leaves it (done, dropped unrun, or panicking), so the layer never waits on
/// one that will not come.
struct Finished(Arc<SunJob>);

impl Drop for Finished {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
    }
}

pub(crate) struct Sunlight {
    pub texture: wgpu::Texture,
    /// The light the texture holds (`None`: lit everywhere, as made without one).
    drawn: Option<Vec3>,
    /// The CPU buffer the sweep writes, kept between computations (lent to a job while it runs).
    bytes: Vec<u8>,
    worker: Option<LandWorker>,
    background: bool,
    job: Option<Arc<SunJob>>,
}

impl Sunlight {
    pub fn new(texture: wgpu::Texture, drawn: Option<Vec3>, bytes: Vec<u8>) -> Self {
        Self {
            texture,
            drawn,
            bytes,
            worker: None,
            background: true,
            job: None,
        }
    }

    pub fn set_worker(&mut self, worker: Option<LandWorker>) {
        self.worker = worker;
    }

    pub fn set_background(&mut self, background: bool) {
        self.background = background;
    }

    /// Whether a shadow is being computed on the worker (or computed and not yet uploaded).
    pub fn pending(&self) -> bool {
        self.job.is_some()
    }

    /// The light the texture holds, or will once the job on the worker lands.
    pub fn light(&self) -> Option<Vec3> {
        self.job.as_ref().map(|job| job.direction).or(self.drawn)
    }

    /// Follows `light` (unit, toward the light): uploads a shadow the worker has finished; then nothing while the light
    /// stays within [`SUN_TOLERANCE`] of the shadow drawn or being computed; else the shadow for it, on the worker when
    /// there is one and background work is on.
    pub fn follow(&mut self, light: Vec3, field: &Arc<PreparedField>, queue: &wgpu::Queue) {
        if let Some(job) = self.job.take_if(|job| job.finished.load(Ordering::Acquire)) {
            match job.bytes.lock().unwrap_or_else(|e| e.into_inner()).take() {
                Some(bytes) => {
                    write_levels(queue, &self.texture, &[&bytes]);
                    self.drawn = Some(job.direction);
                    self.bytes = bytes;
                }
                // The worker never ran it (or it panicked): this thread computes it, now.
                None => self.compute(job.direction, field, queue),
            }
        }
        if self.job.is_some() {
            if self.background {
                return;
            }
            // Background work went off with a job out (a manual clock leaving the Map view): it is let go (it writes no
            // texture), and this thread computes the light now, so the scripted frames that follow repeat.
            self.job = None;
        }
        if self
            .drawn
            .is_some_and(|drawn| light.angle_between(drawn) < SUN_TOLERANCE)
        {
            return;
        }
        // A land with no shadow yet (made without the scene's light) computes its first here: lit everywhere until a
        // worker's landed would show.
        if self.background
            && self.drawn.is_some()
            && let Some(worker) = &self.worker
        {
            let job = Arc::new(SunJob {
                direction: light,
                finished: AtomicBool::new(false),
                bytes: Mutex::new(None),
            });
            self.job = Some(job.clone());
            let finished = Finished(job);
            let mut bytes = std::mem::take(&mut self.bytes);
            let field = field.clone();
            worker(Box::new(move || {
                let direction = finished.0.direction;
                let direction = [
                    f64::from(direction.x),
                    f64::from(direction.y),
                    f64::from(direction.z),
                ];
                field.sunlight(direction, &mut bytes);
                *finished.0.bytes.lock().unwrap_or_else(|e| e.into_inner()) = Some(bytes);
                drop(finished);
            }));
            return;
        }
        self.compute(light, field, queue);
    }

    /// The shadow for `light` on this thread.
    fn compute(&mut self, light: Vec3, field: &PreparedField, queue: &wgpu::Queue) {
        self.drawn = Some(light);
        let direction = [f64::from(light.x), f64::from(light.y), f64::from(light.z)];
        field.sunlight(direction, &mut self.bytes);
        write_levels(queue, &self.texture, &[&self.bytes]);
    }
}
