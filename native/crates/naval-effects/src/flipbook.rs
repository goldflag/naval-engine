//! Trial look `flipbook` ([`crate::Trial`]): gas puffs drawn from the frames of a baked fluid simulation, blended
//! along the motion between them, in place of the atlas's still puffs (`shaders/flipbook.wgsl`).
//!
//! `native/tools/effects-flipbook/bake.py` marches JangaFX's free EmberGen simulations (CC0) with the gas atlas's
//! own lighting kernels and writes one `.flipbook` per simulation to `assets/effects/flipbook/` (Git LFS); the pack
//! carries them with their light textures BC7 (naval-assets, `Kind::Flipbook`). The host reads them (this crate is
//! portable: no file system) and hands them to [`crate::EffectsLayer::set_flipbooks`] as [`FlipbookData`], from a
//! file's bytes ([`FlipbookData::from_bytes`]) or the pack's. A particle names the [`clip`] it plays in `EffectParticle::flipbook`
//! (0, the default, is a still puff), which reaches the sprite shader in `GasPuff::extra[2]`.
//!
//! Ship fires draw two loops: their flames ([`clip::FIRE`], `flipbook_fires.rs`) and the smoke column over them
//! ([`clip::PLUME`], `fire_batch.rs`).
//!
//! A sprite shows one side of its simulation whichever way it is seen, so a puff circled by the eye would turn with it.
//! The fireball's later frames are baked from a second side too ([`SIDE_BOOKS`]); with the look `views` on, a puff
//! playing them keeps a heading of its own and shows the two sides nearest the eye's bearing on it, blended, the far
//! two sides being the near two mirrored (`flipbook_views` in the shader).

use bytemuck::{Pod, Zeroable};

use crate::gpu::sampler;

/// Clips the shader's table holds, and the layers its textures may have (wgpu's default limit).
pub const MAX_CLIPS: usize = 8;
pub const MAX_LAYERS: usize = 256;

/// Where a grounded simulation's ground lies under its tile's centre, in half tiles: the bake leaves a margin of
/// 0.06 round the gas (`MARGIN` in `pack.py`). A puff playing a grounded clip stands on its foot when its centre is
/// this share of its radius above it.
pub const GROUND_ANCHOR: f64 = 1.0 / 1.06;

/// Simulations baked from a second side as well: the book a clip plays, and the book of the same simulation seen from
/// ninety degrees round to the right of it (`bake.py`'s `fireball-side`, marched `--view 1` and framed as the first
/// book frames its gas, so the two match in size and place). A side book holds its first book's last frames, as many
/// as it has.
pub const SIDE_BOOKS: [(&str, &str); 1] = [("fireball", "fireball-side")];

/// Layers over which a puff turns from its first book's side to the eye's, from the first layer the side book holds:
/// a fireball's early frames are seen from one side, its smoke from every side (`VIEWS_RAMP` in the shader).
pub const VIEWS_RAMP: f32 = 4.0;

/// The clips a particle may play, by the number it carries (`EffectParticle::flipbook`).
pub mod clip {
    /// A burst on a surface: the fireball heaving up off it, its stem, its turn to soot and the smoke thinning
    /// away. Stands upright on its foot.
    pub const BLAST: f64 = 1.0;
    /// A fireball in the open: a ball of flame rolling into a ragged cloud of soot. Turns freely.
    pub const FIREBALL: f64 = 2.0;
    /// A billow of smoke: the open fireball's cloud as its flame goes out, rolling and thinning.
    pub const SMOKE: f64 = 3.0;
    /// [`BLAST`] for a burst so large it unfolds slowly (a magazine's).
    pub const BLAST_SLOW: f64 = 4.0;
    /// A fire that burns on: a fuel fire's flame and the soot rolling off it, as a loop. Stands upright on its foot,
    /// turning about the vertical to face the lens (`flipbook_fires.rs`).
    pub const FIRE: f64 = 5.0;
    /// The smoke above a fire: a stretch of a fuel fire's plume, rolling up its tile, as a loop. A link of the chain
    /// a fire's column is drawn as (`fire_batch.rs`): it lies along the column and thins to nothing at both ends.
    pub const PLUME: f64 = 6.0;
}

/// Seconds the fire's loop takes at the simulation's own pace: 64 frames, each two of its thirtieths of a second.
pub const FIRE_LOOP_SECONDS: f64 = 64.0 * 2.0 / 30.0;

/// The plume's tile: its width over its length (the bake's `--span 240:320`), and the share of its length the smoke
/// climbs in one turn of the loop (`pack.py` prints it).
pub const PLUME_ASPECT: f64 = 0.75;
pub const PLUME_CLIMB: f64 = 0.44;

/// A stretch of a simulation a puff plays over its life.
struct ClipDef {
    book: &'static str,
    /// The stretch, as shares of the simulation's frames.
    from: f32,
    to: f32,
    /// How much faster it plays early in the puff's life (`1 − e^(−k t)`); 0 plays it evenly.
    curve: f32,
    /// A fire: the stretch is a loop (its last frame flows into its first, and the puff's life is its place in the
    /// loop), the sprite stands upright on its foot and its flame's heat is the simulation's all the way down.
    fire: bool,
    /// A link of a smoke column: a loop too, its sprite laid along the column (the puff's `quad.xyz`).
    column: bool,
}

/// The clips, in [`clip`]'s order from 1.
const CLIPS: [ClipDef; 6] = [
    ClipDef {
        book: "blast",
        from: 0.04,
        to: 1.0,
        curve: 4.5,
        fire: false,
        column: false,
    },
    ClipDef {
        book: "fireball",
        from: 0.12,
        to: 1.0,
        curve: 3.0,
        fire: false,
        column: false,
    },
    ClipDef {
        book: "fireball",
        from: 0.45,
        to: 0.85,
        curve: 1.0,
        fire: false,
        column: false,
    },
    ClipDef {
        book: "blast",
        from: 0.04,
        to: 1.0,
        curve: 6.0,
        fire: false,
        column: false,
    },
    ClipDef {
        book: "fire",
        from: 0.0,
        to: 1.0,
        curve: 0.0,
        fire: true,
        column: false,
    },
    ClipDef {
        book: "plume",
        from: 0.0,
        to: 1.0,
        curve: 0.0,
        fire: false,
        column: true,
    },
];

/// How a [`FlipbookTexture`]'s texels are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexelFormat {
    /// Four bytes a texel (`Rgba8Unorm`).
    Rgba8,
    /// BC7, 16 bytes a 4 × 4 block (`Bc7RgbaUnorm`; needs `TEXTURE_COMPRESSION_BC`). A level smaller than a block
    /// is one padded block.
    Bc7,
}

impl TexelFormat {
    fn wgpu(self) -> wgpu::TextureFormat {
        match self {
            TexelFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            TexelFormat::Bc7 => wgpu::TextureFormat::Bc7RgbaUnorm,
        }
    }

    /// Texels across a block, and bytes a block.
    fn block(self) -> (u32, usize) {
        match self {
            TexelFormat::Rgba8 => (1, 4),
            TexelFormat::Bc7 => (4, 16),
        }
    }
}

/// One of a simulation's three textures, every frame at every mip level: `levels[l]` is level `l` of each frame in
/// turn (rows from the bottom). Levels halve each frame on its own down to one texel, a 2 × 2 box in the stored
/// values (no sRGB).
#[derive(Clone, Debug, PartialEq)]
pub struct FlipbookTexture {
    pub format: TexelFormat,
    /// Texels across level 0 (a power of two).
    pub edge: u32,
    pub levels: Vec<Vec<u8>>,
}

impl FlipbookTexture {
    /// RGBA8 frames of `edge` texels, one after another, with their mip chain built.
    pub fn rgba8(base: &[u8], edge: u32, frames: usize) -> Self {
        Self {
            format: TexelFormat::Rgba8,
            edge,
            levels: mip_chain(base, edge, frames),
        }
    }

    /// Texels across level `level`.
    pub fn level_edge(&self, level: usize) -> u32 {
        (self.edge >> level).max(1)
    }

    /// Bytes of one frame at level `level`.
    pub fn frame_len(&self, level: usize) -> usize {
        let (block, bytes) = self.format.block();
        (self.level_edge(level).div_ceil(block) as usize).pow(2) * bytes
    }

    /// Whether it holds `frames` frames at every level down to one texel; the error says what is wrong.
    pub fn check(&self, frames: usize) -> Result<(), String> {
        if !self.edge.is_power_of_two() {
            return Err(format!("{} texels across", self.edge));
        }
        let wanted = self.edge.ilog2() as usize + 1;
        if self.levels.len() != wanted {
            return Err(format!(
                "{} mip levels for {} texels (wanted {wanted})",
                self.levels.len(),
                self.edge
            ));
        }
        for (level, data) in self.levels.iter().enumerate() {
            if data.len() != frames * self.frame_len(level) {
                return Err(format!(
                    "level {level} holds {} bytes, not {frames} frames of {}",
                    data.len(),
                    self.frame_len(level)
                ));
            }
        }
        Ok(())
    }

    /// Bytes of every level.
    pub fn bytes(&self) -> u64 {
        self.levels.iter().map(|level| level.len() as u64).sum()
    }
}

/// One baked simulation, as `pack.py` writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct FlipbookData {
    /// The file's stem: what [`CLIPS`] know it by.
    pub name: String,
    /// Texels across a frame, and across a frame's motion.
    pub tile: u32,
    pub motion_tile: u32,
    pub frames: u32,
    /// Where the simulation's ground is under the tile's centre, in half tiles (0 for one in the open).
    pub anchor: f32,
    /// The share of the tile a motion channel's ±1 spans.
    pub motion_range: f32,
    /// The tile's width over its height where it stands in the world (1 for a square one).
    pub aspect: f32,
    /// Each frame's gas as a share of the last frame's size; for a side book ([`SIDE_BOOKS`]), its tile's width over
    /// the first book's tile's instead.
    pub scales: Vec<f32>,
    /// Light from ±X and ±Y.
    pub sides: FlipbookTexture,
    /// Light from the front and from behind, the root of optical depth ÷ 12, the flame seen.
    pub body: FlipbookTexture,
    /// Where a texel of the next frame was in this one (rg) and where a texel of this one is in the next (ba).
    pub motion: FlipbookTexture,
}

impl FlipbookData {
    const HEADER: usize = 32;

    /// The start of a Git LFS pointer: what a checkout holds in place of a book whose LFS object was not fetched.
    pub const LFS_POINTER: &[u8] = b"version https://git-lfs.github.com/spec/";

    /// A `.flipbook` file's bytes (RGBA8 frames; their mip levels are built here); the error says what is wrong with
    /// them.
    pub fn from_bytes(name: &str, bytes: &[u8]) -> Result<Self, String> {
        if bytes.starts_with(Self::LFS_POINTER) {
            return Err(format!(
                "{name}: a Git LFS pointer, not the book (fetch it: git lfs pull --include 'assets/effects/flipbook/*')"
            ));
        }
        if bytes.len() < Self::HEADER || &bytes[..4] != b"NFLP" {
            return Err(format!("{name}: not a flipbook"));
        }
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        let float =
            |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        if word(4) != 1 {
            return Err(format!(
                "{name}: flipbook version {}, this build reads 1",
                word(4)
            ));
        }
        let (tile, frames, motion_tile) = (word(8), word(12), word(16));
        if !tile.is_power_of_two()
            || !motion_tile.is_power_of_two()
            || frames == 0
            || frames as usize > MAX_LAYERS
        {
            return Err(format!(
                "{name}: {frames} frames of {tile} texels (motion {motion_tile})"
            ));
        }
        let (frame, motion) = (
            (tile * tile * 4) as usize,
            (motion_tile * motion_tile * 4) as usize,
        );
        let count = frames as usize;
        let expected = Self::HEADER + count * (4 + frame * 2 + motion);
        if bytes.len() != expected {
            return Err(format!(
                "{name}: {} bytes, its header says {expected}",
                bytes.len()
            ));
        }
        let scales = (0..count).map(|i| float(Self::HEADER + i * 4)).collect();
        let sides = Self::HEADER + count * 4;
        let body = sides + count * frame;
        let moved = body + count * frame;
        Ok(Self {
            name: name.to_owned(),
            tile,
            motion_tile,
            frames,
            anchor: float(20),
            motion_range: float(24),
            // The bake wrote nothing here before it packed fires: a square tile.
            aspect: if float(28) > 0.0 { float(28) } else { 1.0 },
            scales,
            sides: FlipbookTexture::rgba8(&bytes[sides..body], tile, count),
            body: FlipbookTexture::rgba8(&bytes[body..moved], tile, count),
            motion: FlipbookTexture::rgba8(&bytes[moved..], motion_tile, count),
        })
    }

    /// Whether its fields and textures agree; the error says what does not.
    pub fn check(&self) -> Result<(), String> {
        let frames = self.frames as usize;
        if frames == 0 || frames > MAX_LAYERS || self.scales.len() != frames {
            return Err(format!(
                "{}: {} frames, {} scales",
                self.name,
                self.frames,
                self.scales.len()
            ));
        }
        for (what, texture, edge) in [
            ("sides", &self.sides, self.tile),
            ("body", &self.body, self.tile),
            ("motion", &self.motion, self.motion_tile),
        ] {
            if texture.edge != edge {
                return Err(format!(
                    "{}: {what} is {} texels across, the book says {edge}",
                    self.name, texture.edge
                ));
            }
            texture
                .check(frames)
                .map_err(|e| format!("{}: {what}: {e}", self.name))?;
        }
        Ok(())
    }

    /// Bytes of texture it holds, every level.
    pub fn texture_bytes(&self) -> u64 {
        self.sides.bytes() + self.body.bytes() + self.motion.bytes()
    }
}

/// `Flipbooks` in `shaders/flipbook.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FlipbookUniform {
    clips: [[f32; 4]; MAX_CLIPS],
    motion: [[f32; 4]; MAX_CLIPS],
    scales: [[f32; 4]; MAX_LAYERS / 4],
    views: [[f32; 4]; MAX_CLIPS],
}

/// One clip's stretch of layers, as the table holds it: its first and last layer, how fast it plays early in the
/// puff's life, where its ground lies under the tile's centre (half tiles), and whether it loops.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ClipSpan {
    first: f32,
    last: f32,
    curve: f32,
    anchor: f32,
    looped: bool,
}

/// The clip table on the CPU, for what is worked out there of a puff's drawn gas (the smoke's shadow balls,
/// `shadow.rs`): each clip's stretch of layers and every layer's size. Empty (every puff its full size) until
/// simulations are loaded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipSizes {
    clips: [Option<ClipSpan>; MAX_CLIPS],
    scales: Vec<f32>,
}

impl ClipSizes {
    /// The table for `books` loaded as layers in the order given (as [`Flipbooks::new`] loads them).
    pub fn new(books: &[FlipbookData]) -> Self {
        let mut sizes = Self::default();
        let mut first = 0usize;
        for book in books {
            for (index, def) in CLIPS
                .iter()
                .enumerate()
                .filter(|(_, def)| def.book == book.name)
            {
                let span = book.frames.saturating_sub(1) as f32;
                sizes.clips[index] = Some(ClipSpan {
                    first: first as f32 + (def.from * span).round(),
                    last: first as f32 + (def.to * span).round(),
                    curve: def.curve,
                    anchor: book.anchor,
                    looped: def.fire || def.column,
                });
            }
            sizes.scales.extend_from_slice(&book.scales);
            first += book.frames as usize;
        }
        sizes
    }

    /// Whether `clip`'s simulation is loaded.
    pub fn has(&self, clip: f64) -> bool {
        clip >= 0.5
            && self.clips[((clip + 0.5) as usize).saturating_sub(1).min(MAX_CLIPS - 1)].is_some()
    }

    /// The gas a puff playing `clip` draws at `life` (0–1 of its life), as `flipbook_puff` in `shaders/flipbook.wgsl`
    /// finds it: its size as a share of the full-grown puff's, and where its ground lies under the tile's centre (half
    /// tiles; 0 for a clip centred on its gas). (1, 0) for a still puff or a clip whose simulation is not loaded.
    pub fn frame(&self, clip: f64, life: f64) -> (f64, f64) {
        if clip < 0.5 {
            return (1.0, 0.0);
        }
        let index = ((clip + 0.5) as usize).saturating_sub(1).min(MAX_CLIPS - 1);
        let Some(span) = self.clips[index] else {
            return (1.0, 0.0);
        };
        let scale = |layer: f32| f64::from(self.scales.get(layer as usize).copied().unwrap_or(1.0));
        let (position, layer, next) = if span.looped {
            let position = span.first + (life.fract() as f32) * (span.last - span.first + 1.0);
            let layer = position.floor().min(span.last);
            let next = if layer + 1.0 > span.last {
                span.first
            } else {
                layer + 1.0
            };
            (position, layer, next)
        } else {
            let t = life.clamp(0.0, 1.0) as f32;
            let played = if span.curve > 0.01 {
                (1.0 - (-span.curve * t).exp()) / (1.0 - (-span.curve).exp())
            } else {
                t
            };
            let position = span.first + played * (span.last - span.first);
            let layer = position.floor().min(span.last);
            (position, layer, (layer + 1.0).min(span.last))
        };
        let share = f64::from(position - layer);
        (
            scale(layer) + (scale(next) - scale(layer)) * share,
            f64::from(span.anchor),
        )
    }
}

#[cfg(test)]
impl ClipSizes {
    /// Every clip drawing its gas at `scale` of the full-grown puff's size, on a ground `anchor` half tiles under its
    /// centre.
    pub(crate) fn even(scale: f32, anchor: f32) -> Self {
        let span = ClipSpan {
            first: 0.0,
            last: 0.0,
            curve: 0.0,
            anchor,
            looped: false,
        };
        Self {
            clips: [Some(span); MAX_CLIPS],
            scales: vec![scale],
        }
    }
}

/// The simulations on the GPU: every frame a layer of three array textures, and the clip table. Cloning them shares
/// the textures (they never change once uploaded).
#[derive(Clone)]
pub struct Flipbooks {
    pub sides: wgpu::TextureView,
    pub body: wgpu::TextureView,
    pub motion: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub uniform: wgpu::Buffer,
    /// Bytes of texture held, mips included (0 for the stand-in).
    pub bytes: u64,
    /// Whether the fire's loop ([`clip::FIRE`]) is among the simulations.
    pub fire: bool,
    /// Whether the smoke column's loop ([`clip::PLUME`]) is.
    pub plume: bool,
    /// The clip table on the CPU.
    pub sizes: ClipSizes,
}

/// Every mip level of `layers` square RGBA8 frames of `edge` texels, each level's frames one after another: a
/// 2 × 2 box per level.
fn mip_chain(base: &[u8], edge: u32, layers: usize) -> Vec<Vec<u8>> {
    let mut levels = vec![base.to_vec()];
    let mut size = edge as usize;
    while size > 1 {
        let half = size / 2;
        let above = levels.last().expect("the base");
        let mut level = vec![0u8; layers * half * half * 4];
        for layer in 0..layers {
            let source = &above[layer * size * size * 4..(layer + 1) * size * size * 4];
            let target = &mut level[layer * half * half * 4..(layer + 1) * half * half * 4];
            for y in 0..half {
                for x in 0..half {
                    for channel in 0..4 {
                        let at = |dx: usize, dy: usize| {
                            u32::from(source[((y * 2 + dy) * size + x * 2 + dx) * 4 + channel])
                        };
                        target[(y * half + x) * 4 + channel] =
                            ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                    }
                }
            }
        }
        levels.push(level);
        size = half;
    }
    levels
}

/// Every book's `texture` as one array texture, the books' frames one after another (`stand_in`, one frame of one
/// texel, without books); the error names a book whose texture differs from the first's in size or format.
fn array_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    books: &[FlipbookData],
    stand_in: [u8; 4],
    texture: impl Fn(&FlipbookData) -> &FlipbookTexture,
) -> Result<(wgpu::TextureView, u64), String> {
    let stand_in = FlipbookTexture::rgba8(&stand_in, 1, 1);
    let first = books.first().map_or(&stand_in, &texture);
    let (edge, format) = (first.edge, first.format);
    let layers = books.iter().map(|book| book.frames).sum::<u32>().max(1);
    let mut levels: Vec<Vec<u8>> = vec![Vec::new(); first.levels.len()];
    if books.is_empty() {
        levels = stand_in.levels.clone();
    }
    for book in books {
        let own = texture(book);
        if own.edge != edge || own.format != format {
            return Err(format!(
                "{label}: {} is {:?} of {} texels beside {format:?} of {edge}",
                book.name, own.format, own.edge
            ));
        }
        own.check(book.frames as usize)
            .map_err(|e| format!("{label}: {}: {e}", book.name))?;
        for (level, data) in own.levels.iter().enumerate() {
            levels[level].extend_from_slice(data);
        }
    }
    let (block, block_bytes) = format.block();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: edge,
            height: edge,
            depth_or_array_layers: layers,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: format.wgpu(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut bytes = 0;
    for (level, data) in levels.iter().enumerate() {
        // A compressed level is copied whole blocks at a time: one padded block below 4 texels.
        let blocks = ((edge >> level).max(1)).div_ceil(block);
        let side = blocks * block;
        bytes += data.len() as u64;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(blocks * block_bytes as u32),
                rows_per_image: Some(blocks),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: layers,
            },
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    Ok((view, bytes))
}

/// The clip table for `books` loaded as layers in the order given: the CPU's and the shader's. The error names a side
/// book ([`SIDE_BOOKS`]) that does not fit the book it turns.
fn table(books: &[FlipbookData]) -> Result<(ClipSizes, FlipbookUniform), String> {
    let sizes = ClipSizes::new(books);
    let mut uniform = FlipbookUniform::zeroed();
    for (index, span) in sizes.clips.iter().enumerate() {
        // A clip without its simulation: last layer below first.
        uniform.clips[index] = span.map_or([0.0, -1.0, 0.0, 0.0], |span| {
            [span.first, span.last, span.curve, span.anchor]
        });
    }
    for (layer, scale) in sizes.scales.iter().enumerate() {
        uniform.scales[layer / 4][layer % 4] = *scale;
    }
    for book in books {
        for (index, def) in CLIPS
            .iter()
            .enumerate()
            .filter(|(_, def)| def.book == book.name)
        {
            uniform.motion[index] = [
                book.motion_range,
                book.tile as f32,
                // 1 a fire's loop, 2 a column's.
                f32::from(u8::from(def.fire)) + 2.0 * f32::from(u8::from(def.column)),
                book.aspect,
            ];
        }
    }
    // Each book's first layer.
    let mut firsts = Vec::with_capacity(books.len());
    let mut first = 0u32;
    for book in books {
        firsts.push((book, first));
        first += book.frames;
    }
    let find = |name: &str| firsts.iter().find(|(book, _)| book.name == name).copied();
    for (front, side) in SIDE_BOOKS {
        let (Some((front_book, front_first)), Some((side_book, side_first))) =
            (find(front), find(side))
        else {
            continue;
        };
        let (front_frames, side_frames) = (front_book.frames, side_book.frames);
        if side_book.anchor != front_book.anchor {
            return Err(format!("{side}: its ground is not {front}'s"));
        }
        if side_frames > front_frames {
            return Err(format!(
                "{side}: {side_frames} frames, more than the {front_frames} of {front}, whose last frames it holds"
            ));
        }
        // The first of the first book's layers the side book holds, how far on its own layer lies, the last, and the
        // share of the tile the side's motion channels span.
        let base = front_first + front_frames - side_frames;
        let views = [
            side_first as f32 - base as f32,
            base as f32,
            (front_first + front_frames - 1) as f32,
            side_book.motion_range,
        ];
        for (index, def) in CLIPS.iter().enumerate() {
            if def.book == front && !def.fire && !def.column {
                uniform.views[index] = views;
            }
        }
    }
    Ok((sizes, uniform))
}

impl Flipbooks {
    /// No simulations: one empty texel a texture and a table without clips, so every puff is a still one.
    pub fn none(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::build(device, queue, &[]).expect("the stand-in is well formed")
    }

    /// The simulations as layers, in the order given; clips whose simulation is missing stay off (their puffs are
    /// still ones). Every book's textures must share their sizes and formats (BC7 needs a device with
    /// `TEXTURE_COMPRESSION_BC`). The error names what does not fit.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        books: &[FlipbookData],
    ) -> Result<Self, String> {
        Self::build(device, queue, books)
    }

    fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        books: &[FlipbookData],
    ) -> Result<Self, String> {
        let (tile, motion_tile) = books
            .first()
            .map_or((1, 1), |book| (book.tile, book.motion_tile));
        let layers: usize = books.iter().map(|book| book.frames as usize).sum();
        if layers > MAX_LAYERS {
            return Err(format!(
                "{layers} frames in all; the layers hold {MAX_LAYERS}"
            ));
        }
        if let Some(odd) = books
            .iter()
            .find(|book| book.tile != tile || book.motion_tile != motion_tile)
        {
            return Err(format!(
                "{}: frames of {} texels beside frames of {tile}",
                odd.name, odd.tile
            ));
        }
        for book in books {
            book.check()?;
        }
        let (sizes, uniform) = table(books)?;
        let (sides, sides_bytes) =
            array_texture(device, queue, "flipbook sides", books, [0; 4], |b| &b.sides)?;
        let (body, body_bytes) =
            array_texture(device, queue, "flipbook body", books, [0; 4], |b| &b.body)?;
        let (motion, motion_bytes) =
            array_texture(device, queue, "flipbook motion", books, [128; 4], |b| {
                &b.motion
            })?;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flipbook clips"),
            size: std::mem::size_of::<FlipbookUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&uniform));
        Ok(Self {
            sides,
            body,
            motion,
            sampler: sampler(device, "flipbook", wgpu::AddressMode::ClampToEdge, true, 1),
            uniform: buffer,
            bytes: if books.is_empty() {
                0
            } else {
                sides_bytes + body_bytes + motion_bytes
            },
            fire: CLIPS
                .iter()
                .any(|def| def.fire && books.iter().any(|book| book.name == def.book)),
            plume: CLIPS
                .iter()
                .any(|def| def.column && books.iter().any(|book| book.name == def.book)),
            sizes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(frames: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"NFLP");
        for word in [1u32, 2, frames, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(&0.5f32.to_le_bytes());
        bytes.extend_from_slice(&0.1f32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        for frame in 0..frames {
            bytes.extend_from_slice(&((frame + 1) as f32 / frames as f32).to_le_bytes());
        }
        bytes.extend(std::iter::repeat_n(7u8, (frames * (16 + 16 + 4)) as usize));
        bytes
    }

    #[test]
    fn a_flipbook_reads_back_what_the_bake_wrote() {
        let data = FlipbookData::from_bytes("blast", &book(3)).unwrap();
        assert_eq!((data.tile, data.frames, data.motion_tile), (2, 3, 1));
        assert_eq!((data.anchor, data.motion_range), (0.5, 0.1));
        assert_eq!(data.aspect, 1.0);
        assert_eq!(data.scales.len(), 3);
        // Every level of every frame: 2 × 2 and 1 × 1 for the sides and body, 1 × 1 for the motion.
        assert_eq!(
            (data.sides.bytes(), data.body.bytes(), data.motion.bytes()),
            (60, 60, 12)
        );
        assert_eq!(data.sides.levels[0].len(), 48);
        assert!(data.check().is_ok());
        let mut short = data.clone();
        short.body.levels[1].pop();
        assert!(short.check().is_err());
        let pointer = b"version https://git-lfs.github.com/spec/v1\noid sha256:00\nsize 3\n";
        assert!(
            FlipbookData::from_bytes("lfs", pointer)
                .unwrap_err()
                .contains("LFS pointer")
        );
        assert!(FlipbookData::from_bytes("short", &book(3)[..40]).is_err());
        assert!(
            FlipbookData::from_bytes("other", b"nothing of the kind, thirty-two bytes").is_err()
        );
    }

    #[test]
    fn the_clip_table_finds_a_puffs_drawn_gas_as_the_shader_does() {
        // A blast of 26 frames, then a fire of 4 (layers 26–29); each book's frames grow to its full size.
        let blast = FlipbookData::from_bytes("blast", &book(26)).unwrap();
        let fire = FlipbookData::from_bytes("fire", &book(4)).unwrap();
        let sizes = ClipSizes::new(&[blast, fire]);
        // A still puff, and a clip whose simulation is not loaded (the fireball's), are their full size.
        assert_eq!(sizes.frame(0.0, 0.5), (1.0, 0.0));
        assert_eq!(sizes.frame(clip::FIREBALL, 0.5), (1.0, 0.0));
        // So a fire column's billows (the fireball book's smoke) are not drawn.
        assert!(sizes.has(clip::BLAST) && sizes.has(clip::FIRE));
        assert!(!sizes.has(clip::SMOKE) && !sizes.has(clip::PLUME) && !sizes.has(0.0));
        // The blast plays its frames 1 to 25, fast at first (1 − e^(−4.5 t)): its first frame's gas at birth and its
        // last at the end, standing on the book's foot.
        let (born, anchor) = sizes.frame(clip::BLAST, 0.0);
        assert!((born - 2.0 / 26.0).abs() < 1e-6);
        assert_eq!(anchor, 0.5);
        assert!((sizes.frame(clip::BLAST, 1.0).0 - 1.0).abs() < 1e-6);
        let played = (1.0 - (-4.5f64 * 0.2).exp()) / (1.0 - (-4.5f64).exp());
        let position = 1.0 + played * 24.0;
        assert!((sizes.frame(clip::BLAST, 0.2).0 - (position + 1.0) / 26.0).abs() < 1e-4);
        // The fire's loop: its place in the loop, its last frame running back into its first, and the next turn the
        // same.
        assert!((sizes.frame(clip::FIRE, 0.5).0 - 0.75).abs() < 1e-6);
        assert!((sizes.frame(clip::FIRE, 0.9).0 - (1.0 + (0.25 - 1.0) * 0.6)).abs() < 1e-5);
        assert!((sizes.frame(clip::FIRE, 1.5).0 - 0.75).abs() < 1e-6);
    }

    #[test]
    fn a_side_book_turns_the_last_frames_of_its_first_books_clips() {
        // A blast of 4 frames (layers 0–3), the fireball of 6 (4–9) and its last 3 from the side (10–12).
        let blast = FlipbookData::from_bytes("blast", &book(4)).unwrap();
        let fireball = FlipbookData::from_bytes("fireball", &book(6)).unwrap();
        let side = FlipbookData::from_bytes("fireball-side", &book(3)).unwrap();
        let (_, uniform) = table(&[blast.clone(), fireball.clone(), side.clone()]).unwrap();
        // The side's frame lies 3 layers on from the fireball's layers 7–9, and its motion spans the side book's range.
        for clip in [clip::FIREBALL, clip::SMOKE] {
            assert_eq!(uniform.views[clip as usize - 1], [3.0, 7.0, 9.0, 0.1]);
        }
        // The blast has no second side, nor has the fireball without its side book.
        assert_eq!(uniform.views[clip::BLAST as usize - 1], [0.0; 4]);
        let (_, alone) = table(&[blast, fireball.clone()]).unwrap();
        assert!(alone.views.iter().all(|row| *row == [0.0; 4]));
        // A side book longer than its first book does not fit it.
        let long = FlipbookData::from_bytes("fireball-side", &book(7)).unwrap();
        let Err(error) = table(&[fireball, long]) else {
            panic!("a side book longer than its first book was taken");
        };
        assert!(error.contains("fireball-side"), "{error}");
    }

    #[test]
    fn mips_halve_each_frame_on_its_own() {
        // Two 2 × 2 frames, one all 8 and one all 200: each level keeps them apart.
        let base: Vec<u8> = [vec![8u8; 16], vec![200u8; 16]].concat();
        let levels = mip_chain(&base, 2, 2);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1], [vec![8u8; 4], vec![200u8; 4]].concat());
    }
}
