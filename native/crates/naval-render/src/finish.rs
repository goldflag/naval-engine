//! A frame's command encoders finished off the thread that records them.
//!
//! wgpu does most of a frame's CPU work when an encoder finishes: it validates and encodes every pass then, and the
//! driver records its command lists. On a fleet's frame that was over a quarter of the main thread, so
//! [`crate::Renderer::render`] hands each part of its frame to a [`Finisher`] as soon as the part is recorded (the
//! layers' prepare, the shadow maps, each group of the scene pass), and the parts finish beside one another and beside
//! the rest of the frame's recording. The frame's last part finishes where it is submitted, after the others, in one
//! submission in their order.
//!
//! This crate spawns no threads: the client installs a finisher with its own ([`crate::Renderer::set_finisher`]).
//! Without one the frame is one encoder, as before. The parts come back through one channel the renderer keeps, so
//! handing them over allocates nothing frame to frame.

use std::sync::mpsc::{Receiver, Sender};

/// Finishes command encoders on other threads.
pub trait Finisher: Send + Sync {
    /// Finishes `part` ([`Part::finish`]) on another thread, or here if none is left.
    fn finish(&self, part: Part);
}

/// A part's index in its frame, and its command buffer (`None` if it was dropped unfinished).
type Finished = (usize, Option<wgpu::CommandBuffer>);

/// One part of a frame, handed to a [`Finisher`].
pub struct Part {
    encoder: Option<wgpu::CommandEncoder>,
    index: usize,
    done: Sender<Finished>,
    sent: bool,
}

impl Part {
    /// Finishes the part's encoder and hands its command buffer back to the frame.
    pub fn finish(mut self) {
        let encoder = self.encoder.take().expect("a part is finished once");
        let _ = self.done.send((self.index, Some(encoder.finish())));
        self.sent = true;
    }
}

impl Drop for Part {
    /// A part dropped unfinished (its thread panicked while finishing it, say) comes back empty, so the frame fails
    /// rather than waiting for it.
    fn drop(&mut self) {
        if !self.sent {
            let _ = self.done.send((self.index, None));
        }
    }
}

/// The parts a frame handed over, and where they come back; kept from frame to frame.
pub(crate) struct Parts {
    done: Sender<Finished>,
    finished: Receiver<Finished>,
    handed: usize,
    buffers: Vec<Option<wgpu::CommandBuffer>>,
}

impl Parts {
    pub fn new() -> Self {
        let (done, finished) = std::sync::mpsc::channel();
        Self {
            done,
            finished,
            handed: 0,
            buffers: Vec::new(),
        }
    }

    /// Hands the frame's encoder so far to `finisher`, if there is one, and goes on in a fresh one.
    pub fn hand_over(
        &mut self,
        device: &wgpu::Device,
        finisher: Option<&std::sync::Arc<dyn Finisher>>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let Some(finisher) = finisher else {
            return;
        };
        let part = std::mem::replace(
            encoder,
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            }),
        );
        finisher.finish(Part {
            encoder: Some(part),
            index: self.handed,
            done: self.done.clone(),
            sent: false,
        });
        self.handed += 1;
    }

    /// Waits for the parts handed over since the last call and drains their command buffers, in the order they were
    /// handed over.
    pub fn collect(&mut self) -> impl Iterator<Item = wgpu::CommandBuffer> + '_ {
        self.buffers.clear();
        self.buffers.resize_with(self.handed, || None);
        for _ in 0..self.handed {
            let (index, buffer) = self.finished.recv().expect("the parts keep a sender");
            assert!(buffer.is_some(), "a frame part was dropped unfinished");
            self.buffers[index] = buffer;
        }
        self.handed = 0;
        self.buffers.drain(..).flatten()
    }

    /// Drops what a frame that failed before its submission handed over.
    pub fn forget(&mut self) {
        for _ in 0..self.handed {
            let _ = self.finished.recv();
        }
        self.handed = 0;
    }
}
