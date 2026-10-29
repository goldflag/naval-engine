//! The frame's buffer uploads through a staging belt that keeps its memory from frame to frame.
//!
//! `Queue::write_buffer` makes a fresh staging buffer for every call (wgpu-core's `StagingBuffer::new`: on Metal a new
//! `MTLBuffer`, at least one page) and frees it once the GPU has copied from it, so every frame touched fresh pages
//! and faulted them in: about 190 faults (3 MB) a frame in the 30-ship battle (`native/docs/load.md`,
//! `native/docs/frame-hygiene.md`). [`Uploads`] writes into chunks of a [`wgpu::util::StagingBelt`] instead: mapped
//! memory that is copied from and mapped again once the GPU is done with it, so a steady frame reuses the same pages.
//!
//! The ordering is `Queue::write_buffer`'s: every upload is recorded into the belt's own encoder, which the renderer
//! submits first in the frame's submission ([`Uploads::finish`]), so the data is in place before any pass of that
//! submission reads it, wherever in the frame the write was made.
//!
//! On Windows (DX12) a `write_buffer` costs more than the pages: each call's staging buffer is a driver resource made
//! and freed through the allocator, about 7 % of the client's main thread in a theatre (2026-10-09). So the small
//! uniform writes a frame makes every frame go through the belt too, not only the large ones.
//!
//! A pass drawn after the frame in a submission of its own (the page's composite, the tags' and the maps' text)
//! cannot use the frame's [`Uploads`], whose encoder goes with the next frame: [`PassUploads`] records its writes into
//! that pass's own encoder, ahead of the pass.

/// The unit the belt allocates its staging memory in: above most single writes (a larger one gets a chunk of its own
/// size, kept for reuse) and a fraction of a frame's uploads.
const CHUNK_BYTES: u64 = 1 << 20;

/// Uploads gathered over one frame, submitted ahead of it.
pub struct Uploads {
    device: wgpu::Device,
    belt: wgpu::util::StagingBelt,
    encoder: Option<wgpu::CommandEncoder>,
    /// Writes and bytes since the last [`Uploads::finish`].
    writes: u32,
    bytes: u64,
}

/// What one frame uploaded through the belt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadStats {
    pub writes: u32,
    pub bytes: u64,
}

impl Uploads {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            device: device.clone(),
            belt: wgpu::util::StagingBelt::new(device.clone(), CHUNK_BYTES),
            encoder: None,
            writes: 0,
            bytes: 0,
        }
    }

    /// `Queue::write_buffer` through the belt: `data` lands in `target` at `offset` before the commands of the next
    /// submission that carries [`Uploads::finish`]'s command buffer. As there, `offset` and `data.len()` are multiples
    /// of [`wgpu::COPY_BUFFER_ALIGNMENT`]; an empty write does nothing.
    pub fn write_buffer(
        &mut self,
        target: &wgpu::Buffer,
        offset: wgpu::BufferAddress,
        data: &[u8],
    ) {
        let Some(size) = wgpu::BufferSize::new(data.len() as u64) else {
            return;
        };
        let device = &self.device;
        let encoder = self.encoder.get_or_insert_with(|| {
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("uploads"),
            })
        });
        self.belt
            .write_buffer(encoder, target, offset, size)
            .copy_from_slice(data);
        self.writes += 1;
        self.bytes += size.get();
    }

    /// A copy of `size` bytes from the start of `source` to the start of `target` on the GPU, in order with the
    /// belt's writes: one made before this lands before the copy, one after it after. A size of 0 does nothing.
    pub fn copy_buffer(
        &mut self,
        source: &wgpu::Buffer,
        target: &wgpu::Buffer,
        size: wgpu::BufferAddress,
    ) {
        if size == 0 {
            return;
        }
        let device = &self.device;
        self.encoder
            .get_or_insert_with(|| {
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("uploads"),
                })
            })
            .copy_buffer_to_buffer(source, 0, target, 0, size);
    }

    /// The uploads since the last call, as a command buffer to submit before the frame's own (`None`: there were
    /// none). Call [`Uploads::recall`] after that submission.
    pub fn finish(&mut self) -> Option<wgpu::CommandBuffer> {
        let encoder = self.encoder.take()?;
        self.belt.finish();
        Some(encoder.finish())
    }

    /// After the submission that carried [`Uploads::finish`]'s command buffer: the belt's chunks are mapped again,
    /// for reuse once the GPU has copied from them.
    pub fn recall(&mut self) {
        self.belt.recall();
    }

    /// Writes and bytes recorded since the last [`Uploads::finish`], and resets the count (telemetry).
    pub fn take_stats(&mut self) -> UploadStats {
        let stats = UploadStats {
            writes: self.writes,
            bytes: self.bytes,
        };
        self.writes = 0;
        self.bytes = 0;
        stats
    }
}

/// Uploads for a pass submitted on its own, after the frame (the page's composite, the tags' text): recorded into the
/// pass's encoder ahead of the pass, through a staging belt of their own that keeps its memory from frame to frame.
///
/// The data lands where the copy is recorded in the encoder, so a write made before the pass is recorded is in place
/// for it, as `Queue::write_buffer`'s would be for a pass that is the submission's only reader of the buffer.
pub struct PassUploads {
    belt: wgpu::util::StagingBelt,
    /// A write since the last [`PassUploads::finish`].
    written: bool,
}

impl PassUploads {
    /// A belt allocating `chunk_bytes` at a time (at most the device's largest buffer): above the pass's usual writes
    /// in one frame.
    pub fn new(device: &wgpu::Device, chunk_bytes: u64) -> Self {
        let chunk_bytes = chunk_bytes.min(device.limits().max_buffer_size);
        Self {
            belt: wgpu::util::StagingBelt::new(device.clone(), chunk_bytes),
            written: false,
        }
    }

    /// `Queue::write_buffer` into `encoder`: `data` lands in `target` at `offset` before the commands recorded on
    /// `encoder` after this. As there, `offset` and `data.len()` are multiples of [`wgpu::COPY_BUFFER_ALIGNMENT`]; an
    /// empty write does nothing. Call [`PassUploads::finish`] before `encoder` is finished.
    pub fn write_buffer(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::Buffer,
        offset: wgpu::BufferAddress,
        data: &[u8],
    ) {
        let Some(size) = wgpu::BufferSize::new(data.len() as u64) else {
            return;
        };
        self.belt
            .write_buffer(encoder, target, offset, size)
            .copy_from_slice(data);
        self.written = true;
    }

    /// `Queue::write_texture` into `encoder`: `data`, laid out as `layout` says, lands in `target` before the commands
    /// recorded on `encoder` after this, so a pass recorded earlier on it still reads what was there. Unlike
    /// `write_texture`, `layout.bytes_per_row` is a multiple of [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`] (a copy from a
    /// buffer's rule). An empty write does nothing. Call [`PassUploads::finish`] before `encoder` is finished.
    pub fn write_texture(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: wgpu::TexelCopyTextureInfo<'_>,
        data: &[u8],
        layout: wgpu::TexelCopyBufferLayout,
        size: wgpu::Extent3d,
    ) {
        let Some(len) = wgpu::BufferSize::new(
            (data.len() as u64).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT),
        ) else {
            return;
        };
        let align = wgpu::BufferSize::new(u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
            .expect("the row alignment is not zero");
        let slice = self.belt.allocate(len, align);
        slice
            .get_mapped_range_mut()
            .expect("a staging belt's slice is mapped")
            .slice(..data.len())
            .copy_from_slice(data);
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: slice.buffer(),
                layout: wgpu::TexelCopyBufferLayout {
                    offset: slice.offset() + layout.offset,
                    ..layout
                },
            },
            target,
            size,
        );
        self.written = true;
    }

    /// After the writes into `encoder`, before it is finished: the belt's memory is mapped again for reuse once
    /// `encoder`'s submission has run. An encoder dropped unsubmitted takes the memory it held with it.
    pub fn finish(&mut self, encoder: &wgpu::CommandEncoder) {
        if std::mem::take(&mut self.written) {
            self.belt.finish_and_recall_on_submit(encoder);
        }
    }
}

/// Where a layer's buffer writes go: straight to the queue (`write_buffer`, a staging buffer per call: fine for the
/// odd small write, for tests and stills), or through the frame's [`Uploads`].
pub enum UploadSink<'a> {
    Queue(&'a wgpu::Queue),
    Belt(&'a mut Uploads),
}

impl UploadSink<'_> {
    /// `Queue::write_buffer`, through whichever path this sink is.
    pub fn write_buffer(
        &mut self,
        target: &wgpu::Buffer,
        offset: wgpu::BufferAddress,
        data: &[u8],
    ) {
        match self {
            Self::Queue(queue) => queue.write_buffer(target, offset, data),
            Self::Belt(uploads) => uploads.write_buffer(target, offset, data),
        }
    }

    /// [`Uploads::copy_buffer`] through the belt, true; through the queue nothing, false: the queue's writes land
    /// before any copy the frame records, so the caller writes instead.
    pub fn copy_buffer(
        &mut self,
        source: &wgpu::Buffer,
        target: &wgpu::Buffer,
        size: wgpu::BufferAddress,
    ) -> bool {
        match self {
            Self::Queue(_) => false,
            Self::Belt(uploads) => {
                uploads.copy_buffer(source, target, size);
                true
            }
        }
    }
}
