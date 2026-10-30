//! The facades' textures (native/docs/terrain/buildings.md, "Facade"): drawn here when the layer is made on High and
//! Ultra (nothing to fetch or license), as texture arrays with their mip chains, so a wall far off takes its windows'
//! mean without moiré.
//!
//! - **Walls** ([`Material`], [`WALL_TEXELS`] square, [`WALL_TILE_M`] metres a side, laid along a wall in metres):
//!   render, stucco, brick, lap siding, boards and battens, corrugated sheet, concrete panels, ashlar, ceramic tile. A
//!   texel: r the shade on the wall's palette colour (0.5 leaves it), g the joints (mortar, grout, a board's shadow
//!   line), b and a the surface's normal across the wall and up it (0.5 flat).
//! - **Windows** ([`Window`], [`WINDOW_TEXELS`] square, one bay by one storey, the first row the storey's top): frames,
//!   glass, sills, shutters, doors, balconies, a shop's front, and a past period's (carriage doors, shoji and amado, a
//!   masonry block's punched window, awnings and signboards, wooden louvres). Two textures: the colour (linear) premultiplied by its
//!   cover, and the cover (0 where the wall shows through); and the normal (across, up), the glass (lit at night, the
//!   sky in it) and the share the building's accent colour tints (shutters, doors, a shop's sign, a curtain wall's
//!   glass), the colour there a grey the accent multiplies (0.5 leaves it). The surface array's second half holds each
//!   window's recess, layer [`Window::ALL`]'s length on (r: 1 where it lies back in the wall's opening: its frames,
//!   glass and reveal, not the trim, sill, surround, shutters or rails proud of the wall), for the shader's parallax and
//!   the reveal's shade.
//!
//! The windows' order is `buildings.wgsl`'s `WINDOW_*` constants', the materials' its `MATERIAL_*` (a test holds them
//! together).

/// A wall material's texels a side.
pub const WALL_TEXELS: u32 = 256;
/// The metres a wall material's tile spans each way.
pub const WALL_TILE_M: f32 = 4.0;
/// A window's texels a side.
pub const WINDOW_TEXELS: u32 = 128;
/// The metres a window's cell is taken to span each way, for its relief's slopes (a bay is 3-3.3 m, a storey 3 m).
const CELL_M: f32 = 3.0;
/// Samples a texel each way.
const WALL_SAMPLES: u32 = 2;
const WINDOW_SAMPLES: u32 = 4;

/// A wall's surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    Render,
    Stucco,
    Brick,
    Siding,
    Boards,
    Corrugated,
    Panels,
    Ashlar,
    Tile,
}

impl Material {
    pub const ALL: [Material; 9] = [
        Material::Render,
        Material::Stucco,
        Material::Brick,
        Material::Siding,
        Material::Boards,
        Material::Corrugated,
        Material::Panels,
        Material::Ashlar,
        Material::Tile,
    ];

    /// Its constant's name in `buildings.wgsl`.
    pub fn name(self) -> &'static str {
        match self {
            Material::Render => "RENDER",
            Material::Stucco => "STUCCO",
            Material::Brick => "BRICK",
            Material::Siding => "SIDING",
            Material::Boards => "BOARDS",
            Material::Corrugated => "CORRUGATED",
            Material::Panels => "PANELS",
            Material::Ashlar => "ASHLAR",
            Material::Tile => "TILE",
        }
    }
}

/// A window's cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    /// A white sash window, two over two, under a head and over a sill (temperate, American).
    Sash,
    /// The sash between louvred shutters (American).
    SashShuttered,
    /// A tall French window between wide louvred shutters (the Mediterranean, the tropics).
    Shuttered,
    /// A tall casement in a stone surround (the Mediterranean).
    Casement,
    /// A white cross-framed window in a wide white trim (Nordic).
    Nordic,
    /// A wide, low aluminium slider in stucco (the American West).
    Slider,
    /// An aluminium sash under a small rain hood (a Japanese house).
    JapanHouse,
    /// Glass louvres in a painted frame (the tropics).
    Jalousie,
    /// A small deep window behind an iron grille (a dry country).
    Grille,
    /// A panelled front door under a fanlight, a step before it.
    Door,
    /// A sectional garage door.
    Garage,
    /// A block's window: a fixed light between two openers under a transom.
    Block,
    /// A French door onto a balcony: its slab, iron rail and balusters.
    Balcony,
    /// A Japanese block's balcony: a solid parapet before sliding glass doors.
    JapanBalcony,
    /// A Japanese block's aluminium window under its hood.
    JapanBlock,
    /// A shop's front: glass from the pavement under its sign.
    Shopfront,
    /// An office's curtain wall: glass between mullions, a spandrel at the floor.
    Curtain,
    /// An office's ribbon of glass across the bay.
    Ribbon,
    /// A factory's high ribbon of windows.
    HighRibbon,
    /// A warehouse's roller shutter door.
    Roller,
    /// A church's tall arched window of leaded glass.
    Arched,
    /// A belfry's louvred arch.
    Belfry,
    // A past period's (native/docs/terrain/period.md, "Period styling"):
    /// Wooden carriage doors: two boarded leaves on strap hinges, a row of small lights over them (a house's coach
    /// house, a warehouse's doors).
    CarriageDoor,
    /// A Japanese house's wooden glazed sliding doors: four leaves of small lights over paper panels.
    Shoji,
    /// A Japanese house's amado: wooden storm shutters closed over the opening.
    Amado,
    /// A masonry block's or office's punched window: a six-over-six sash in a stone surround under its lintel.
    Punched,
    /// A shop's front under a striped canvas awning: a wooden frame, its stall riser below.
    Awning,
    /// A shop's front under a painted signboard: small panes in a wooden frame.
    Signboard,
    /// Wooden louvred shutters under a glazed fanlight (the tropics).
    Louvres,
    /// An aluminium slider behind an iron cage (鐵窗) standing out from the wall under a small sheet hood, painted in
    /// the building's accent (white, cream, rust or grey): Taiwan's homes.
    Cage,
}

impl Window {
    pub const ALL: [Window; 30] = [
        Window::Sash,
        Window::SashShuttered,
        Window::Shuttered,
        Window::Casement,
        Window::Nordic,
        Window::Slider,
        Window::JapanHouse,
        Window::Jalousie,
        Window::Grille,
        Window::Door,
        Window::Garage,
        Window::Block,
        Window::Balcony,
        Window::JapanBalcony,
        Window::JapanBlock,
        Window::Shopfront,
        Window::Curtain,
        Window::Ribbon,
        Window::HighRibbon,
        Window::Roller,
        Window::Arched,
        Window::Belfry,
        Window::CarriageDoor,
        Window::Shoji,
        Window::Amado,
        Window::Punched,
        Window::Awning,
        Window::Signboard,
        Window::Louvres,
        Window::Cage,
    ];

    /// Its constant's name in `buildings.wgsl`.
    pub fn name(self) -> &'static str {
        match self {
            Window::Sash => "SASH",
            Window::SashShuttered => "SASH_SHUTTERED",
            Window::Shuttered => "SHUTTERED",
            Window::Casement => "CASEMENT",
            Window::Nordic => "NORDIC",
            Window::Slider => "SLIDER",
            Window::JapanHouse => "JAPAN_HOUSE",
            Window::Jalousie => "JALOUSIE",
            Window::Grille => "GRILLE",
            Window::Door => "DOOR",
            Window::Garage => "GARAGE",
            Window::Block => "BLOCK",
            Window::Balcony => "BALCONY",
            Window::JapanBalcony => "JAPAN_BALCONY",
            Window::JapanBlock => "JAPAN_BLOCK",
            Window::Shopfront => "SHOPFRONT",
            Window::Curtain => "CURTAIN",
            Window::Ribbon => "RIBBON",
            Window::HighRibbon => "HIGH_RIBBON",
            Window::Roller => "ROLLER",
            Window::Arched => "ARCHED",
            Window::Belfry => "BELFRY",
            Window::CarriageDoor => "CARRIAGE_DOOR",
            Window::Shoji => "SHOJI",
            Window::Amado => "AMADO",
            Window::Punched => "PUNCHED",
            Window::Awning => "AWNING",
            Window::Signboard => "SIGNBOARD",
            Window::Louvres => "LOUVRES",
            Window::Cage => "CAGE",
        }
    }
}

/// One texture array: `size` texels square, `layers` deep, each mip level's RGBA8 texels layer after layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Layers {
    pub size: u32,
    pub layers: u32,
    pub levels: Vec<Vec<u8>>,
}

impl Layers {
    /// Bytes on the GPU.
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }

    /// Level 0's texel (`x`, `y`) of layer `layer`.
    pub fn texel(&self, layer: u32, x: u32, y: u32) -> [u8; 4] {
        let i = (((layer * self.size + y) * self.size + x) * 4) as usize;
        let t = &self.levels[0][i..i + 4];
        [t[0], t[1], t[2], t[3]]
    }
}

/// The facades' textures.
#[derive(Clone, Debug, PartialEq)]
pub struct Facades {
    /// [`Material::ALL`]'s layers.
    pub walls: Layers,
    /// [`Window::ALL`]'s layers: colour and cover; normal, glass and accent, then each window's recess.
    pub windows: Layers,
    pub window_surface: Layers,
}

impl Facades {
    pub fn bytes(&self) -> usize {
        self.walls.bytes() + self.windows.bytes() + self.window_surface.bytes()
    }
}

/// The textures, drawn the first time they are asked for and kept (a scene made again takes them as they are).
pub fn baked() -> &'static Facades {
    static FACADES: std::sync::OnceLock<Facades> = std::sync::OnceLock::new();
    FACADES.get_or_init(bake)
}

/// Draws every layer with its mips.
pub fn bake() -> Facades {
    let (mut walls, mut windows, mut surface, mut recess) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for m in Material::ALL {
        walls.extend(wall_layer(m));
    }
    for w in Window::ALL {
        let (colour, rest, back) = window_layer(w);
        windows.extend(colour);
        surface.extend(rest);
        recess.extend(back);
    }
    surface.extend(recess);
    Facades {
        walls: with_mips(walls, WALL_TEXELS, Material::ALL.len() as u32),
        windows: with_mips(windows, WINDOW_TEXELS, Window::ALL.len() as u32),
        window_surface: with_mips(surface, WINDOW_TEXELS, 2 * Window::ALL.len() as u32),
    }
}

/// Every level from `base` (`layers` layers of `size` texels square): each a 2 × 2 box of the one above (the
/// textures repeat, so a level's texels never straddle a layer's edge).
fn with_mips(base: Vec<u8>, size: u32, layers: u32) -> Layers {
    let mut levels = vec![base];
    let mut s = size;
    while s > 1 {
        let h = s / 2;
        let above = levels.last().unwrap();
        let mut next = vec![0u8; (h * h * layers * 4) as usize];
        for l in 0..layers {
            for y in 0..h {
                for x in 0..h {
                    for c in 0..4 {
                        let at = |xx: u32, yy: u32| {
                            u32::from(above[(((l * s + yy) * s + xx) * 4 + c) as usize])
                        };
                        let sum = at(2 * x, 2 * y)
                            + at(2 * x + 1, 2 * y)
                            + at(2 * x, 2 * y + 1)
                            + at(2 * x + 1, 2 * y + 1);
                        next[(((l * h + y) * h + x) * 4 + c) as usize] = ((sum + 2) / 4) as u8;
                    }
                }
            }
        }
        levels.push(next);
        s = h;
    }
    Layers {
        size,
        layers,
        levels,
    }
}

fn unorm(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// An integer hash to 0-1.
fn hash(x: i32, y: i32, k: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ k.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// Value noise about -0.5-0.5 with cells `cell` metres across, repeating every [`WALL_TILE_M`].
fn noise(x: f32, y: f32, cell: f32, k: u32) -> f32 {
    streaked(x, y, cell, cell, k)
}

/// Value noise with cells `across` by `up` metres (a grain drawn out up a board), repeating every [`WALL_TILE_M`].
fn streaked(x: f32, y: f32, across: f32, up: f32, k: u32) -> f32 {
    let nx = (WALL_TILE_M / across).round().max(1.0) as i32;
    let ny = (WALL_TILE_M / up).round().max(1.0) as i32;
    let (fx, fy) = (x / WALL_TILE_M * nx as f32, y / WALL_TILE_M * ny as f32);
    let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
    let (tx, ty) = (fx - ix as f32, fy - iy as f32);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let at = |i: i32, j: i32| hash(i.rem_euclid(nx), j.rem_euclid(ny), k);
    let a = at(ix, iy) + (at(ix + 1, iy) - at(ix, iy)) * sx;
    let b = at(ix, iy + 1) + (at(ix + 1, iy + 1) - at(ix, iy + 1)) * sx;
    a + (b - a) * sy - 0.5
}

/// A wall material at (`x`, `y`) metres (`y` down from the tile's top): its shade (0.5 leaves the palette's colour),
/// joints (0-1) and relief (m, out of the wall).
fn wall_sample(m: Material, x: f32, y: f32) -> (f32, f32, f32) {
    // The rows a tile holds; a running bond's alternate rows set over by half a unit.
    let units = |rows: f32, per_row: f32, joint: f32, k: u32| {
        let h = WALL_TILE_M / rows;
        let w = WALL_TILE_M / per_row;
        let row = (y / h).floor();
        let shift = if (row as i32).rem_euclid(2) == 1 {
            0.5 * w
        } else {
            0.0
        };
        let col = ((x + shift) / w).floor();
        let (fx, fy) = ((x + shift) - col * w, y - row * h);
        let edge = fx.min(w - fx).min(fy).min(h - fy);
        let joint_share = 1.0 - smooth(joint * 0.5, joint * 0.5 + 0.004, edge);
        let unit = hash(
            (col as i32).rem_euclid(per_row as i32),
            (row as i32).rem_euclid(rows as i32),
            k,
        );
        (unit, joint_share, edge)
    };
    match m {
        Material::Render => {
            let n = noise(x, y, 0.25, 1) * 0.6 + noise(x, y, 1.0, 2) * 0.4;
            let fine = noise(x, y, 0.0625, 3);
            (0.5 + 0.05 * n + 0.02 * fine, 0.0, 0.0012 * fine)
        }
        Material::Stucco => {
            let grain = noise(x, y, 0.03125, 4) * 0.6 + noise(x, y, 0.0625, 5) * 0.4;
            let patch = noise(x, y, 0.5, 6) * 0.6 + noise(x, y, 2.0, 7) * 0.4;
            (0.5 + 0.06 * patch + 0.04 * grain, 0.0, 0.003 * grain)
        }
        Material::Brick => {
            let (unit, joint, edge) = units(54.0, 18.0, 0.008, 8);
            // Each brick its own: a little lighter or darker, one in nine a dark clinker.
            let clinker = if hash((unit * 977.0) as i32, 3, 9) < 0.11 {
                -0.12
            } else {
                0.0
            };
            let shade = 0.5 + 0.1 * (unit - 0.5) + clinker + 0.03 * noise(x, y, 0.0625, 10);
            (
                shade,
                joint,
                -0.006 * joint + 0.0015 * smooth(0.0, 0.01, edge),
            )
        }
        Material::Siding => {
            // Lap boards of 0.2 m: each thicker at its foot, a dark line in the shadow beneath it.
            let board = 0.2;
            let row = (y / board).floor();
            let t = (y - row * board) / board;
            let line = 1.0 - smooth(0.92, 0.97, t);
            let per = hash((row as i32).rem_euclid(20), 0, 11);
            let grain = noise(x, y, 0.0625, 12) * 0.5 + streaked(x, y, 1.0, 0.25, 13) * 0.5;
            (
                0.5 + 0.03 * (per - 0.5) + 0.02 * grain,
                1.0 - line,
                0.012 * t,
            )
        }
        Material::Boards => {
            // Boards 0.25 m wide, battens 0.05 m over the joints, the grain running up.
            let w = 0.25;
            let col = (x / w).floor();
            let fx = x - col * w;
            let batten_d = fx.min(w - fx);
            let batten = 1.0 - smooth(0.022, 0.028, batten_d);
            let grain = streaked(x, y, 0.0625, 0.5, 14);
            let per = hash((col as i32).rem_euclid(16), 0, 15);
            let edge = smooth(0.0, 0.006, (0.025 - batten_d).abs());
            (
                0.5 + 0.06 * (per - 0.5) + 0.05 * grain,
                0.35 * (1.0 - edge),
                0.015 * batten,
            )
        }
        Material::Corrugated => {
            // Ribs of 77 mm running up, sheets lapped every 2 m.
            let pitch = WALL_TILE_M / 52.0;
            let phase = x / pitch * std::f32::consts::TAU;
            let lap = y.rem_euclid(2.0).min(2.0 - y.rem_euclid(2.0));
            let line = 1.0 - smooth(0.004, 0.012, lap);
            (
                0.5 + 0.04 * phase.cos() + 0.015 * noise(x, y, 0.5, 16),
                0.4 * line,
                0.009 * phase.sin(),
            )
        }
        Material::Panels => {
            // Precast panels of 2 by 1.33 m, recessed joints, tie holes on a 0.67 m grid, mottled.
            let (pw, ph) = (2.0, WALL_TILE_M / 3.0);
            let (fx, fy) = (x.rem_euclid(pw), y.rem_euclid(ph));
            let edge = fx.min(pw - fx).min(fy).min(ph - fy);
            let joint = 1.0 - smooth(0.008, 0.014, edge);
            let panel = hash(
                ((x / pw).floor() as i32).rem_euclid(2),
                ((y / ph).floor() as i32).rem_euclid(3),
                17,
            );
            let tie_m = WALL_TILE_M / 6.0;
            let (tx, ty) = (
                x.rem_euclid(tie_m) - tie_m * 0.5,
                y.rem_euclid(tie_m) - tie_m * 0.5,
            );
            let tie = 1.0 - smooth(0.008, 0.014, (tx * tx + ty * ty).sqrt());
            let mottle = noise(x, y, 0.25, 18) * 0.6 + noise(x, y, 1.0, 19) * 0.4;
            (
                0.5 + 0.06 * (panel - 0.5) + 0.05 * mottle - 0.12 * tie,
                joint,
                -0.01 * joint - 0.004 * tie,
            )
        }
        Material::Ashlar => {
            let (unit, joint, edge) = units(12.0, 6.0, 0.012, 20);
            let face = noise(x, y, 0.0625, 21);
            (
                0.5 + 0.12 * (unit - 0.5) + 0.04 * face,
                joint,
                -0.006 * joint + 0.004 * smooth(0.0, 0.03, edge) + 0.0015 * face,
            )
        }
        Material::Tile => {
            // Glazed tiles of 95 by 45 mm in a running bond, light grout.
            let (unit, joint, _) = units(80.0, 40.0, 0.005, 22);
            (0.5 + 0.08 * (unit - 0.5), joint, -0.003 * joint)
        }
    }
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The normal's across and up from a relief `height` (rows top down) of `size` texels square, `texel_m` metres each,
/// repeating; scaled by `strength`.
fn normals(height: &[f32], size: u32, texel_m: f32, strength: f32) -> Vec<[f32; 2]> {
    let s = size as i32;
    let at = |x: i32, y: i32| height[(y.rem_euclid(s) * s + x.rem_euclid(s)) as usize];
    let mut out = Vec::with_capacity(height.len());
    for y in 0..s {
        for x in 0..s {
            let dx = (at(x + 1, y) - at(x - 1, y)) / (2.0 * texel_m) * strength;
            // Rows run down: up the wall is toward the row above.
            let dy = (at(x, y - 1) - at(x, y + 1)) / (2.0 * texel_m) * strength;
            let l = (dx * dx + dy * dy + 1.0).sqrt();
            out.push([-dx / l, -dy / l]);
        }
    }
    out
}

fn wall_layer(m: Material) -> Vec<u8> {
    let size = WALL_TEXELS;
    let texel = WALL_TILE_M / size as f32;
    let n = WALL_SAMPLES;
    let mut shade = Vec::with_capacity((size * size) as usize);
    let mut joints = Vec::with_capacity((size * size) as usize);
    let mut height = Vec::with_capacity((size * size) as usize);
    for y in 0..size {
        for x in 0..size {
            let (mut s, mut j, mut h) = (0.0, 0.0, 0.0);
            for sy in 0..n {
                for sx in 0..n {
                    let px = (x as f32 + (sx as f32 + 0.5) / n as f32) * texel;
                    let py = (y as f32 + (sy as f32 + 0.5) / n as f32) * texel;
                    let (a, b, c) = wall_sample(m, px, py);
                    s += a;
                    j += b;
                    h += c;
                }
            }
            let k = (n * n) as f32;
            shade.push(s / k);
            joints.push(j / k);
            height.push(h / k);
        }
    }
    let normal = normals(&height, size, texel, 1.0);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for i in 0..(size * size) as usize {
        out.extend([
            unorm(shade[i]),
            unorm(joints[i]),
            unorm(normal[i][0] * 0.5 + 0.5),
            unorm(normal[i][1] * 0.5 + 0.5),
        ]);
    }
    out
}

/// A window cell's texel: its colour (linear), cover, relief (m), glass and accent.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Px {
    rgb: [f32; 3],
    cover: f32,
    height: f32,
    glass: f32,
    accent: f32,
}

fn srgb(r: u8, g: u8, b: u8) -> [f32; 3] {
    let f = |c: u8| {
        let v = f32::from(c) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    [f(r), f(g), f(b)]
}

const fn solid(rgb: [f32; 3], height: f32) -> Px {
    Px {
        rgb,
        cover: 1.0,
        height,
        glass: 0.0,
        accent: 0.0,
    }
}

/// A part the building's accent colour tints: a grey `shade` (0.5 leaves the accent).
const fn accent(shade: f32, height: f32) -> Px {
    Px {
        rgb: [shade; 3],
        cover: 1.0,
        height,
        glass: 0.0,
        accent: 1.0,
    }
}

const GLASS: Px = Px {
    rgb: [0.018, 0.022, 0.028],
    cover: 1.0,
    height: -0.07,
    glass: 1.0,
    accent: 0.0,
};

/// The shade in a window's reveal, under its head.
const REVEAL: Px = solid([0.05, 0.05, 0.05], -0.06);

/// A cell being painted at (`u`, `v`) (0-1 across, down), later shapes over earlier.
struct Cell {
    u: f32,
    v: f32,
    px: Px,
}

impl Cell {
    fn inside(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
        (x0..x1).contains(&self.u) && (y0..y1).contains(&self.v)
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, px: Px) {
        if self.inside(x0, y0, x1, y1) {
            self.px = px;
        }
    }

    /// A frame `t` wide about the rectangle's inside edge.
    fn frame(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, t: f32, px: Px) {
        if self.inside(x0, y0, x1, y1) && !self.inside(x0 + t, y0 + t, x1 - t, y1 - t) {
            self.px = px;
        }
    }

    /// `n` upright bars `t` wide spread evenly between `x0` and `x1` (none at the ends).
    #[allow(clippy::too_many_arguments)]
    fn bars_up(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, n: u32, t: f32, px: Px) {
        for k in 1..=n {
            let x = x0 + (x1 - x0) * k as f32 / (n + 1) as f32;
            self.rect(x - t * 0.5, y0, x + t * 0.5, y1, px);
        }
    }

    /// `n` level bars `t` high spread evenly between `y0` and `y1`.
    #[allow(clippy::too_many_arguments)]
    fn bars_across(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, n: u32, t: f32, px: Px) {
        for k in 1..=n {
            let y = y0 + (y1 - y0) * k as f32 / (n + 1) as f32;
            self.rect(x0, y - t * 0.5, x1, y + t * 0.5, px);
        }
    }

    /// Louvres `pitch` apart: each slat lighter at its top, the shade under it, tilted out.
    fn louvres(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, pitch: f32, px: Px) {
        if self.inside(x0, y0, x1, y1) {
            let t = ((self.v - y0) / pitch).fract();
            let mut p = px;
            for c in &mut p.rgb {
                *c *= 1.15 - 0.45 * t;
            }
            p.height += 0.012 * t;
            self.px = p;
        }
    }

    /// Inside a round-headed arch over `x0`-`x1`, its springing at `spring`, down to `y1`.
    fn arch(&self, x0: f32, spring: f32, x1: f32, y1: f32, top: f32) -> bool {
        if !(x0..x1).contains(&self.u) || self.v > y1 || self.v < top {
            return false;
        }
        if self.v >= spring {
            return true;
        }
        let cx = (x0 + x1) * 0.5;
        let rx = (x1 - x0) * 0.5;
        let ry = spring - top;
        let (dx, dy) = ((self.u - cx) / rx, (self.v - spring) / ry);
        dx * dx + dy * dy <= 1.0
    }
}

/// A sash window over `x0`-`x1`, `y0`-`y1`: its head, sill, white frame, two sashes of two lights.
fn sash(c: &mut Cell, x0: f32, x1: f32, y0: f32, y1: f32, frame: Px, trim: Px) {
    c.rect(x0 - 0.035, y0 - 0.045, x1 + 0.035, y0, trim);
    c.rect(
        x0 - 0.04,
        y1,
        x1 + 0.04,
        y1 + 0.035,
        Px {
            height: 0.04,
            ..trim
        },
    );
    c.rect(x0, y0, x1, y1, frame);
    let ym = (y0 + y1) * 0.5;
    let xm = (x0 + x1) * 0.5;
    for (a, b) in [(y0 + 0.03, ym - 0.012), (ym + 0.012, y1 - 0.03)] {
        c.rect(x0 + 0.03, a, x1 - 0.03, b, GLASS);
        c.rect(xm - 0.008, a, xm + 0.008, b, frame);
    }
}

/// Louvred shutters `w` wide either side of `x0`-`x1`.
fn shutters(c: &mut Cell, x0: f32, x1: f32, y0: f32, y1: f32, w: f32) {
    let leaf = accent(0.5, 0.03);
    for (a, b) in [(x0 - w, x0), (x1, x1 + w)] {
        c.rect(a, y0, b, y1, leaf);
        c.louvres(a + 0.012, y0 + 0.02, b - 0.012, y1 - 0.02, 0.035, leaf);
    }
}

fn window_px(w: Window, u: f32, v: f32) -> Px {
    let white = solid(srgb(234, 234, 228), -0.015);
    let trim = solid(srgb(226, 222, 210), 0.015);
    let stone = solid(srgb(196, 190, 176), 0.02);
    let concrete = solid(srgb(176, 174, 168), 0.0);
    let alu = solid(srgb(178, 180, 180), -0.01);
    let dark_alu = solid(srgb(66, 64, 62), -0.01);
    let iron = solid(srgb(26, 26, 28), 0.08);
    let wood = solid(srgb(112, 78, 52), -0.015);
    let mut c = Cell {
        u,
        v,
        px: Px {
            rgb: [0.0; 3],
            cover: 0.0,
            height: 0.0,
            glass: 0.0,
            accent: 0.0,
        },
    };
    match w {
        Window::Sash => sash(&mut c, 0.3, 0.7, 0.18, 0.7, white, trim),
        Window::SashShuttered => {
            sash(&mut c, 0.31, 0.69, 0.18, 0.7, white, trim);
            shutters(&mut c, 0.31, 0.69, 0.18, 0.7, 0.13);
        }
        Window::Shuttered => {
            c.rect(0.3, 0.11, 0.7, 0.79, stone);
            c.rect(
                0.34,
                0.79,
                0.66,
                0.82,
                Px {
                    height: 0.05,
                    ..stone
                },
            );
            c.rect(0.34, 0.13, 0.66, 0.79, wood);
            c.rect(0.37, 0.16, 0.49, 0.76, GLASS);
            c.rect(0.51, 0.16, 0.63, 0.76, GLASS);
            c.bars_across(0.37, 0.16, 0.63, 0.76, 2, 0.012, wood);
            shutters(&mut c, 0.34, 0.66, 0.13, 0.79, 0.16);
        }
        Window::Casement => {
            c.rect(0.3, 0.1, 0.7, 0.8, stone);
            c.rect(0.33, 0.13, 0.67, 0.8, REVEAL);
            c.rect(0.345, 0.14, 0.655, 0.79, accent(0.35, -0.04));
            c.rect(0.37, 0.165, 0.49, 0.765, GLASS);
            c.rect(0.51, 0.165, 0.63, 0.765, GLASS);
            c.rect(
                0.31,
                0.8,
                0.69,
                0.83,
                Px {
                    height: 0.05,
                    ..stone
                },
            );
        }
        Window::Nordic => {
            c.rect(
                0.24,
                0.16,
                0.76,
                0.74,
                Px {
                    height: 0.02,
                    ..white
                },
            );
            c.rect(0.29, 0.21, 0.71, 0.69, white);
            c.rect(0.31, 0.23, 0.69, 0.67, GLASS);
            c.rect(0.49, 0.23, 0.51, 0.67, white);
            c.rect(0.31, 0.37, 0.69, 0.39, white);
            c.rect(
                0.25,
                0.74,
                0.75,
                0.77,
                Px {
                    height: 0.04,
                    ..white
                },
            );
        }
        Window::Slider => {
            c.rect(0.17, 0.27, 0.83, 0.67, REVEAL);
            c.rect(0.18, 0.28, 0.82, 0.66, alu);
            c.rect(0.195, 0.295, 0.495, 0.645, GLASS);
            c.rect(0.505, 0.295, 0.805, 0.645, GLASS);
        }
        Window::JapanHouse => {
            c.rect(
                0.2,
                0.2,
                0.8,
                0.245,
                Px {
                    height: 0.14,
                    ..dark_alu
                },
            );
            c.rect(0.22, 0.27, 0.78, 0.66, alu);
            c.rect(0.235, 0.285, 0.495, 0.645, GLASS);
            c.rect(0.505, 0.285, 0.765, 0.645, GLASS);
            // The lower lights frosted.
            let frosted = Px {
                rgb: srgb(150, 156, 162),
                glass: 0.6,
                ..GLASS
            };
            c.rect(0.235, 0.5, 0.495, 0.645, frosted);
        }
        Window::Jalousie => {
            c.rect(
                0.28,
                0.18,
                0.72,
                0.72,
                Px {
                    height: 0.02,
                    ..white
                },
            );
            c.rect(0.31, 0.21, 0.69, 0.69, GLASS);
            c.louvres(
                0.31,
                0.21,
                0.69,
                0.69,
                0.04,
                Px {
                    rgb: [0.05, 0.06, 0.07],
                    ..GLASS
                },
            );
            c.bars_across(0.31, 0.21, 0.69, 0.69, 11, 0.006, white);
            c.rect(0.49, 0.21, 0.51, 0.69, white);
        }
        Window::Grille => {
            c.rect(
                0.35,
                0.24,
                0.65,
                0.62,
                Px {
                    height: -0.12,
                    ..REVEAL
                },
            );
            c.rect(
                0.38,
                0.28,
                0.65,
                0.62,
                Px {
                    height: -0.14,
                    ..GLASS
                },
            );
            c.bars_up(0.35, 0.24, 0.65, 0.62, 5, 0.012, iron);
            c.bars_across(0.35, 0.24, 0.65, 0.62, 2, 0.012, iron);
            c.rect(
                0.33,
                0.62,
                0.67,
                0.645,
                Px {
                    height: 0.03,
                    ..stone
                },
            );
        }
        Window::Door => {
            c.rect(0.32, 0.1, 0.68, 0.96, trim);
            c.rect(0.355, 0.13, 0.645, 0.2, GLASS);
            c.rect(0.355, 0.215, 0.645, 0.96, accent(0.5, -0.03));
            for (y0, y1) in [(0.25, 0.55), (0.6, 0.9)] {
                c.frame(0.385, y0, 0.49, y1, 0.012, accent(0.38, -0.04));
                c.frame(0.51, y0, 0.615, y1, 0.012, accent(0.38, -0.04));
            }
            c.rect(
                0.3,
                0.96,
                0.7,
                1.0,
                Px {
                    height: 0.1,
                    ..concrete
                },
            );
        }
        Window::Garage => {
            c.rect(0.08, 0.26, 0.92, 1.0, trim);
            c.rect(0.11, 0.29, 0.89, 1.0, solid(srgb(214, 212, 204), -0.04));
            c.bars_across(
                0.11,
                0.29,
                0.89,
                1.0,
                3,
                0.01,
                solid(srgb(150, 148, 142), -0.05),
            );
        }
        Window::Block => {
            c.rect(0.17, 0.19, 0.83, 0.73, REVEAL);
            c.rect(0.18, 0.2, 0.82, 0.72, white);
            c.rect(0.2, 0.22, 0.8, 0.7, GLASS);
            c.rect(0.39, 0.22, 0.41, 0.7, white);
            c.rect(0.59, 0.22, 0.61, 0.7, white);
            c.rect(0.2, 0.33, 0.8, 0.345, white);
            c.rect(
                0.16,
                0.73,
                0.84,
                0.755,
                Px {
                    height: 0.035,
                    ..stone
                },
            );
        }
        Window::Balcony => {
            c.rect(0.24, 0.08, 0.76, 0.93, REVEAL);
            c.rect(0.25, 0.09, 0.75, 0.93, white);
            c.rect(0.27, 0.11, 0.49, 0.93, GLASS);
            c.rect(0.51, 0.11, 0.73, 0.93, GLASS);
            // The rail and balusters, the wall or the door seen between them.
            c.rect(0.03, 0.6, 0.97, 0.625, iron);
            c.rect(0.03, 0.9, 0.97, 0.915, iron);
            for k in 0..24 {
                let x = 0.05 + 0.9 * k as f32 / 23.0;
                c.rect(x - 0.005, 0.625, x + 0.005, 0.9, iron);
            }
            c.rect(
                0.0,
                0.93,
                1.0,
                1.0,
                Px {
                    height: 0.35,
                    ..concrete
                },
            );
        }
        Window::JapanBalcony => {
            // The slab's shade overhead, sliding doors, the parapet before them.
            c.rect(0.0, 0.0, 1.0, 0.05, solid([0.06, 0.06, 0.06], -0.4));
            c.rect(0.0, 0.05, 1.0, 0.62, solid([0.09, 0.09, 0.09], -1.0));
            c.rect(
                0.08,
                0.1,
                0.92,
                0.62,
                Px {
                    height: -1.0,
                    ..alu
                },
            );
            for (a, b) in [(0.095, 0.29), (0.305, 0.495), (0.505, 0.695), (0.71, 0.905)] {
                c.rect(
                    a,
                    0.115,
                    b,
                    0.62,
                    Px {
                        height: -1.05,
                        ..GLASS
                    },
                );
            }
            c.rect(0.0, 0.6, 1.0, 1.0, solid(srgb(214, 212, 206), 0.15));
            c.rect(0.0, 0.6, 1.0, 0.625, solid(srgb(150, 150, 146), 0.18));
        }
        Window::JapanBlock => {
            c.rect(
                0.16,
                0.18,
                0.84,
                0.225,
                Px {
                    height: 0.12,
                    ..dark_alu
                },
            );
            c.rect(0.2, 0.25, 0.8, 0.68, alu);
            c.rect(0.215, 0.265, 0.495, 0.665, GLASS);
            c.rect(0.505, 0.265, 0.785, 0.665, GLASS);
        }
        Window::Shopfront => {
            c.rect(0.0, 0.02, 1.0, 0.17, accent(0.5, 0.06));
            c.rect(0.0, 0.17, 1.0, 1.0, dark_alu);
            c.rect(0.03, 0.2, 0.97, 0.94, GLASS);
            c.rect(0.49, 0.2, 0.51, 0.94, dark_alu);
            c.rect(
                0.03,
                0.94,
                0.97,
                1.0,
                Px {
                    height: -0.02,
                    ..dark_alu
                },
            );
        }
        Window::Curtain => {
            let glass = Px {
                accent: 1.0,
                rgb: [0.06; 3],
                ..GLASS
            };
            c.rect(0.0, 0.0, 1.0, 1.0, glass);
            let spandrel = Px {
                glass: 0.0,
                height: -0.06,
                rgb: [0.1; 3],
                ..glass
            };
            c.rect(0.0, 0.82, 1.0, 1.0, spandrel);
            let mullion = Px {
                height: 0.03,
                ..dark_alu
            };
            c.rect(0.0, 0.0, 0.02, 1.0, mullion);
            c.rect(0.98, 0.0, 1.0, 1.0, mullion);
            c.rect(0.0, 0.81, 1.0, 0.83, mullion);
        }
        Window::Ribbon => {
            c.rect(0.0, 0.29, 1.0, 0.79, REVEAL);
            c.rect(0.0, 0.3, 1.0, 0.78, GLASS);
            c.rect(0.0, 0.3, 0.012, 0.78, dark_alu);
            c.rect(0.988, 0.3, 1.0, 0.78, dark_alu);
            c.rect(0.494, 0.3, 0.506, 0.78, dark_alu);
            c.rect(0.0, 0.3, 1.0, 0.312, dark_alu);
        }
        Window::HighRibbon => {
            c.rect(
                0.04,
                0.12,
                0.96,
                0.34,
                Px {
                    height: -0.01,
                    ..dark_alu
                },
            );
            c.rect(0.05, 0.135, 0.95, 0.325, GLASS);
            c.bars_up(0.05, 0.135, 0.95, 0.325, 3, 0.012, dark_alu);
        }
        Window::Roller => {
            c.rect(0.1, 0.1, 0.9, 0.16, solid(srgb(150, 152, 152), 0.04));
            c.rect(0.12, 0.16, 0.88, 1.0, accent(0.42, -0.03));
            c.louvres(0.12, 0.16, 0.88, 1.0, 0.03, accent(0.42, -0.03));
            c.rect(0.1, 0.16, 0.12, 1.0, solid(srgb(120, 122, 122), 0.0));
            c.rect(0.88, 0.16, 0.9, 1.0, solid(srgb(120, 122, 122), 0.0));
        }
        Window::Arched => {
            if c.arch(0.33, 0.28, 0.67, 0.86, 0.1) {
                c.px = stone;
            }
            if c.arch(0.36, 0.28, 0.64, 0.84, 0.13) {
                // Leaded lights: diamonds of lead over dark glass.
                let (a, b) = ((u + v * 0.5) * 22.0, (u - v * 0.5) * 22.0);
                let lead = (a - a.round()).abs().min((b - b.round()).abs()) < 0.06;
                c.px = if lead {
                    solid([0.04, 0.04, 0.04], -0.06)
                } else {
                    Px {
                        rgb: [0.03, 0.028, 0.035],
                        ..GLASS
                    }
                };
            }
            c.rect(
                0.32,
                0.84,
                0.68,
                0.87,
                Px {
                    height: 0.05,
                    ..stone
                },
            );
        }
        Window::Belfry => {
            if c.arch(0.3, 0.36, 0.7, 0.82, 0.2) {
                c.px = stone;
            }
            if c.arch(0.33, 0.36, 0.67, 0.8, 0.23) {
                c.px = solid([0.02, 0.02, 0.02], -0.2);
                c.louvres(0.33, 0.3, 0.67, 0.8, 0.06, solid(srgb(70, 62, 54), -0.12));
            }
        }
        Window::CarriageDoor => {
            c.rect(0.08, 0.22, 0.92, 1.0, trim);
            c.rect(0.11, 0.25, 0.89, 1.0, accent(0.48, -0.03));
            // Boards up each leaf, the leaves' meeting, a row of small lights, strap hinges.
            for k in 0..14 {
                let x = 0.11 + 0.78 * k as f32 / 14.0;
                c.rect(x - 0.003, 0.4, x + 0.003, 1.0, accent(0.34, -0.035));
            }
            c.rect(0.495, 0.25, 0.505, 1.0, solid([0.03; 3], -0.05));
            c.rect(0.11, 0.37, 0.89, 0.4, accent(0.4, -0.02));
            for k in 0..6 {
                let x0 = 0.14 + 0.72 * k as f32 / 6.0;
                c.rect(x0 + 0.012, 0.28, x0 + 0.108, 0.355, GLASS);
            }
            for y in [0.5, 0.86] {
                c.rect(0.14, y, 0.46, y + 0.018, iron);
                c.rect(0.54, y, 0.86, y + 0.018, iron);
            }
        }
        Window::Shoji => {
            let dark_wood = solid(srgb(64, 46, 34), 0.0);
            // The lintel over the opening, four leaves of small lights over paper panels between their stiles.
            c.rect(0.08, 0.1, 0.92, 0.86, dark_wood);
            c.rect(
                0.06,
                0.08,
                0.94,
                0.14,
                Px {
                    height: 0.06,
                    ..dark_wood
                },
            );
            let paper = Px {
                rgb: srgb(214, 206, 186),
                cover: 1.0,
                height: -0.03,
                glass: 0.35,
                accent: 0.0,
            };
            for k in 0..4 {
                let x0 = 0.1 + 0.2 * k as f32;
                let x1 = x0 + 0.2;
                c.rect(x0 + 0.018, 0.17, x1 - 0.018, 0.56, GLASS);
                c.bars_across(x0 + 0.018, 0.17, x1 - 0.018, 0.56, 2, 0.01, dark_wood);
                c.bars_up(x0 + 0.018, 0.17, x1 - 0.018, 0.56, 1, 0.01, dark_wood);
                c.rect(x0 + 0.018, 0.6, x1 - 0.018, 0.82, paper);
                c.bars_up(x0 + 0.018, 0.6, x1 - 0.018, 0.82, 3, 0.006, dark_wood);
                c.bars_across(x0 + 0.018, 0.6, x1 - 0.018, 0.82, 2, 0.006, dark_wood);
            }
        }
        Window::Amado => {
            let board = solid(srgb(84, 66, 50), 0.01);
            c.rect(0.08, 0.1, 0.92, 0.86, solid(srgb(56, 42, 32), 0.0));
            c.rect(0.1, 0.15, 0.9, 0.84, board);
            c.bars_up(
                0.1,
                0.15,
                0.9,
                0.84,
                7,
                0.006,
                solid(srgb(46, 36, 28), -0.01),
            );
            c.bars_across(
                0.1,
                0.15,
                0.9,
                0.84,
                1,
                0.012,
                solid(srgb(62, 48, 36), 0.02),
            );
            c.rect(
                0.06,
                0.08,
                0.94,
                0.14,
                Px {
                    height: 0.06,
                    ..solid(srgb(56, 42, 32), 0.0)
                },
            );
        }
        Window::Punched => {
            // The surround and its lintel, the reveal, a white six-over-six sash on a stone sill.
            c.rect(0.27, 0.11, 0.73, 0.82, stone);
            c.rect(
                0.25,
                0.08,
                0.75,
                0.15,
                Px {
                    height: 0.05,
                    ..stone
                },
            );
            c.rect(0.31, 0.16, 0.69, 0.8, REVEAL);
            sash(&mut c, 0.33, 0.67, 0.19, 0.79, white, stone);
            let ym = (0.19 + 0.79) * 0.5;
            for (a, b) in [(0.22, ym - 0.012), (ym + 0.012, 0.76)] {
                c.bars_up(0.36, a, 0.64, b, 2, 0.01, white);
            }
        }
        Window::Awning => {
            // A striped canvas awning over the shop's front, its valance scalloped.
            let cream = solid(srgb(214, 206, 186), 0.25);
            if c.inside(0.0, 0.0, 1.0, 0.3) {
                let stripe = ((c.u * 10.0).floor() as i32).rem_euclid(2) == 0;
                let scallop = c.v > 0.25 && ((c.u * 20.0).fract() - 0.5).abs() * 0.1 + 0.25 < c.v;
                if !scallop {
                    c.px = if stripe { accent(0.5, 0.25) } else { cream };
                    // Lighter toward its top, in the sun; the fold shaded.
                    let shade = 1.05 - 0.25 * (c.v / 0.3);
                    for ch in &mut c.px.rgb {
                        *ch *= shade;
                    }
                }
            }
            if c.inside(0.0, 0.3, 1.0, 1.0) {
                c.px = solid(srgb(60, 44, 32), -0.01);
            }
            c.rect(
                0.05,
                0.33,
                0.95,
                0.8,
                Px {
                    rgb: [0.012, 0.014, 0.018],
                    ..GLASS
                },
            );
            c.rect(0.49, 0.33, 0.51, 0.8, solid(srgb(60, 44, 32), -0.01));
            c.rect(0.05, 0.43, 0.95, 0.445, solid(srgb(60, 44, 32), -0.01));
            c.rect(0.04, 0.83, 0.96, 0.98, accent(0.36, 0.0));
            c.frame(0.07, 0.85, 0.93, 0.96, 0.012, accent(0.28, -0.01));
        }
        Window::Signboard => {
            // A painted signboard under a cornice, its lettering; small panes in a wooden frame, the stall riser.
            c.rect(
                0.0,
                0.0,
                1.0,
                0.05,
                Px {
                    height: 0.06,
                    ..stone
                },
            );
            c.rect(0.0, 0.05, 1.0, 0.21, accent(0.5, 0.03));
            c.frame(
                0.02,
                0.06,
                0.98,
                0.2,
                0.012,
                solid(srgb(200, 190, 160), 0.035),
            );
            for k in 0..11 {
                let x = 0.14 + 0.07 * k as f32;
                c.rect(x, 0.1, x + 0.045, 0.165, solid(srgb(226, 214, 176), 0.03));
            }
            c.rect(0.0, 0.21, 1.0, 1.0, wood);
            c.rect(0.05, 0.25, 0.95, 0.78, GLASS);
            c.bars_up(0.05, 0.25, 0.95, 0.78, 3, 0.012, wood);
            c.bars_across(0.05, 0.25, 0.95, 0.78, 1, 0.012, wood);
            c.rect(0.04, 0.81, 0.96, 0.98, accent(0.4, 0.0));
            c.frame(0.07, 0.83, 0.93, 0.96, 0.012, accent(0.3, -0.01));
        }
        Window::Louvres => {
            // A glazed fanlight over two leaves of wooden louvres, closed, in a white frame on a sill.
            c.rect(
                0.24,
                0.14,
                0.76,
                0.8,
                Px {
                    height: 0.02,
                    ..white
                },
            );
            c.rect(0.28, 0.18, 0.72, 0.34, GLASS);
            c.bars_up(0.28, 0.18, 0.72, 0.34, 2, 0.01, white);
            let leaf = accent(0.5, 0.02);
            for (a, b) in [(0.28, 0.495), (0.505, 0.72)] {
                c.rect(a, 0.36, b, 0.76, leaf);
                c.louvres(a + 0.015, 0.38, b - 0.015, 0.74, 0.03, leaf);
            }
            c.rect(
                0.22,
                0.8,
                0.78,
                0.83,
                Px {
                    height: 0.04,
                    ..white
                },
            );
        }
        Window::Cage => {
            // The slider behind.
            c.rect(0.17, 0.27, 0.83, 0.67, REVEAL);
            c.rect(0.18, 0.28, 0.82, 0.66, alu);
            c.rect(0.195, 0.295, 0.495, 0.645, GLASS);
            c.rect(0.505, 0.295, 0.805, 0.645, GLASS);
            // The cage before it: its frame, close upright bars and three rails, a pressed-sheet band along its foot
            // and a scrolled one under its head, its floor standing out over a sill, and the sheet hood over it. Bars,
            // bands and frame cover about half the cage, so that far off, where a window is its mean, a cage still
            // reads as a pale box over a dark pane rather than as the pane.
            let bar = accent(0.5, 0.12);
            c.frame(0.14, 0.2, 0.86, 0.73, 0.02, bar);
            c.bars_up(0.14, 0.2, 0.86, 0.73, 15, 0.012, bar);
            c.bars_across(0.14, 0.2, 0.86, 0.73, 3, 0.01, bar);
            c.rect(
                0.14,
                0.2,
                0.86,
                0.29,
                Px {
                    height: 0.11,
                    ..bar
                },
            );
            c.rect(
                0.14,
                0.655,
                0.86,
                0.7,
                Px {
                    height: 0.11,
                    ..bar
                },
            );
            c.rect(
                0.13,
                0.715,
                0.87,
                0.74,
                Px {
                    height: 0.14,
                    ..bar
                },
            );
            c.rect(
                0.12,
                0.17,
                0.88,
                0.2,
                Px {
                    height: 0.16,
                    ..alu
                },
            );
        }
    }
    c.px
}

/// Below this relief (m) a window's part lies back in the wall's opening.
const RECESSED_M: f32 = -0.005;

/// A window's three layers: colour premultiplied by cover with the cover; the normal, glass and accent; its recess.
fn window_layer(w: Window) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let size = WINDOW_TEXELS;
    let n = WINDOW_SAMPLES;
    let count = (size * size) as usize;
    let mut sum = vec![
        Px {
            rgb: [0.0; 3],
            cover: 0.0,
            height: 0.0,
            glass: 0.0,
            accent: 0.0,
        };
        count
    ];
    let mut back = vec![0.0f32; count];
    let k = (n * n) as f32;
    for y in 0..size {
        for x in 0..size {
            let b = &mut back[(y * size + x) as usize];
            let s = &mut sum[(y * size + x) as usize];
            for sy in 0..n {
                for sx in 0..n {
                    let u = (x as f32 + (sx as f32 + 0.5) / n as f32) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / n as f32) / size as f32;
                    let p = window_px(w, u, v);
                    for c in 0..3 {
                        s.rgb[c] += p.rgb[c] * p.cover / k;
                    }
                    s.cover += p.cover / k;
                    s.height += p.height * p.cover / k;
                    s.glass += p.glass * p.cover / k;
                    s.accent += p.accent * p.cover / k;
                    if p.cover > 0.0 && p.height < RECESSED_M {
                        *b += p.cover / k;
                    }
                }
            }
        }
    }
    let height: Vec<f32> = sum.iter().map(|p| p.height).collect();
    // Softened: a frame's edge lights, it does not glare.
    let normal = normals(&height, size, CELL_M / size as f32, 0.35);
    let mut colour = Vec::with_capacity(count * 4);
    let mut surface = Vec::with_capacity(count * 4);
    let mut recess = Vec::with_capacity(count * 4);
    for &b in &back {
        recess.extend([unorm(b), 0, 0, 255]);
    }
    for (p, nrm) in sum.iter().zip(&normal) {
        colour.extend([
            unorm(p.rgb[0]),
            unorm(p.rgb[1]),
            unorm(p.rgb[2]),
            unorm(p.cover),
        ]);
        surface.extend([
            unorm(nrm[0] * 0.5 + 0.5),
            unorm(nrm[1] * 0.5 + 0.5),
            unorm(p.glass),
            unorm(p.accent),
        ]);
    }
    (colour, surface, recess)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layers_and_their_mips_are_whole() {
        let started = std::time::Instant::now();
        let f = bake();
        eprintln!(
            "facades baked in {:.0} ms, {:.1} MB",
            started.elapsed().as_secs_f64() * 1e3,
            f.bytes() as f64 / 1048576.0
        );
        for (layers, size, count) in [
            (&f.walls, WALL_TEXELS, Material::ALL.len()),
            (&f.windows, WINDOW_TEXELS, Window::ALL.len()),
            (&f.window_surface, WINDOW_TEXELS, 2 * Window::ALL.len()),
        ] {
            assert_eq!(layers.size, size);
            assert_eq!(layers.layers as usize, count);
            assert_eq!(layers.levels.len() as u32, size.trailing_zeros() + 1);
            for (k, level) in layers.levels.iter().enumerate() {
                let s = (size >> k) as usize;
                assert_eq!(level.len(), s * s * count * 4);
            }
        }
        // Under the brief's 64 MB by far.
        assert!(f.bytes() < 16 << 20, "{} bytes", f.bytes());
    }

    #[test]
    fn windows_hold_glass_in_a_wall_and_the_wall_shows_round_them() {
        let f = bake();
        let s = WINDOW_TEXELS;
        for (k, w) in Window::ALL.iter().enumerate() {
            let k = k as u32;
            let mut cover = 0u32;
            let mut glass = 0u32;
            for y in 0..s {
                for x in 0..s {
                    cover += u32::from(f.windows.texel(k, x, y)[3] > 127);
                    glass += u32::from(f.window_surface.texel(k, x, y)[2] > 127);
                }
            }
            let share = |n: u32| n as f32 / (s * s) as f32;
            match w {
                // Doors, a garage, a roller shutter and closed shutters hold little or no glass; the rest a window's
                // worth.
                Window::Door
                | Window::Garage
                | Window::Roller
                | Window::Belfry
                | Window::CarriageDoor
                | Window::Amado => {
                    assert!(share(glass) < 0.08, "{w:?} {}", share(glass))
                }
                _ => assert!(share(glass) > 0.05, "{w:?} {}", share(glass)),
            }
            // The wall shows round every window but a curtain wall's, a Japanese balcony's and a shop's front.
            match w {
                Window::Curtain
                | Window::JapanBalcony
                | Window::Shopfront
                | Window::Awning
                | Window::Signboard => {
                    assert!(share(cover) > 0.9, "{w:?}")
                }
                _ => assert!(share(cover) < 0.75, "{w:?} {}", share(cover)),
            }
            // A cell's corner is wall, but where the cell's parts run edge to edge.
            if !matches!(
                w,
                Window::Curtain
                    | Window::JapanBalcony
                    | Window::Shopfront
                    | Window::Balcony
                    | Window::Ribbon
                    | Window::Awning
                    | Window::Signboard
            ) {
                assert_eq!(f.windows.texel(k, 1, 1)[3], 0, "{w:?}");
            }
        }
        // Shutters and doors take the building's accent; a sash does not.
        let accent = |w: Window| {
            let k = Window::ALL.iter().position(|x| *x == w).unwrap() as u32;
            (0..s)
                .flat_map(|y| (0..s).map(move |x| (x, y)))
                .filter(|&(x, y)| f.window_surface.texel(k, x, y)[3] > 127)
                .count()
        };
        assert!(accent(Window::SashShuttered) > 500);
        assert!(accent(Window::Door) > 1000);
        assert_eq!(accent(Window::Sash), 0);
        // A past period's awnings, signboards, carriage doors and louvres take it too.
        for w in [
            Window::Awning,
            Window::Signboard,
            Window::CarriageDoor,
            Window::Louvres,
        ] {
            assert!(accent(w) > 500, "{w:?}");
        }
        assert_eq!(accent(Window::Punched), 0);
    }

    #[test]
    fn walls_tile_seamlessly_and_brick_has_its_mortar() {
        let f = bake();
        let s = WALL_TEXELS;
        for (k, m) in Material::ALL.iter().enumerate() {
            let k = k as u32;
            // The tile repeats along and up a wall: each material is the same a tile over (a point or two may fall on a
            // joint's edge in one and not the other).
            let mut off = 0;
            for i in 0..97 {
                for j in 0..89 {
                    let (x, y) = (
                        (i as f32 + 0.37) * WALL_TILE_M / 97.0,
                        (j as f32 + 0.61) * WALL_TILE_M / 89.0,
                    );
                    let a = wall_sample(*m, x, y);
                    for b in [
                        wall_sample(*m, x + WALL_TILE_M, y),
                        wall_sample(*m, x, y + WALL_TILE_M),
                    ] {
                        if (a.0 - b.0).abs() > 0.01
                            || (a.1 - b.1).abs() > 0.05
                            || (a.2 - b.2).abs() > 0.001
                        {
                            off += 1;
                        }
                    }
                }
            }
            assert!(
                off <= 97 * 89 * 2 / 100,
                "{m:?}: {off} points differ a tile over"
            );
            // The shade's mean leaves the palette's colour about as it is.
            let mean = (0..s)
                .flat_map(|y| (0..s).map(move |x| (x, y)))
                .map(|(x, y)| f64::from(f.walls.texel(k, x, y)[0]))
                .sum::<f64>()
                / f64::from(s * s);
            assert!((mean - 127.5).abs() < 14.0, "{m:?} {mean}");
        }
        let brick = Material::ALL
            .iter()
            .position(|m| *m == Material::Brick)
            .unwrap() as u32;
        let mortar = (0..s)
            .flat_map(|y| (0..s).map(move |x| (x, y)))
            .filter(|&(x, y)| f.walls.texel(brick, x, y)[1] > 127)
            .count() as f32
            / (s * s) as f32;
        assert!((0.08..0.3).contains(&mortar), "{mortar}");
    }

    /// FNV-1a over the first `layers` layers of every level.
    fn fingerprint(l: &Layers, layers: u32) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for (k, level) in l.levels.iter().enumerate() {
            let s = (l.size >> k).max(1) as usize;
            for &b in &level[..s * s * 4 * layers as usize] {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }

    #[test]
    fn todays_layers_are_as_they_were() {
        let f = baked();
        assert_eq!(
            [
                fingerprint(&f.walls, 9),
                fingerprint(&f.windows, 22),
                fingerprint(&f.window_surface, 22)
            ],
            [
                12_785_917_216_347_304_827,
                238_321_713_577_218_718,
                14_476_327_835_715_672_560
            ]
        );
    }

    /// A WGSL function's text, from its `fn` line to the next top-level `}`.
    fn function<'a>(shader: &'a str, name: &str) -> &'a str {
        let start = shader
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("{name}"));
        let end = shader[start..].find("\n}\n").unwrap() + start;
        &shader[start..end]
    }

    #[test]
    fn a_past_period_draws_no_modern_windows_or_walls() {
        let shader = include_str!("../../shaders/buildings.wgsl");
        let windows = function(shader, "window_style_past");
        for modern in [
            Window::Slider,
            Window::JapanHouse,
            Window::Jalousie,
            Window::Garage,
            Window::JapanBalcony,
            Window::JapanBlock,
            Window::Shopfront,
            Window::Curtain,
            Window::Ribbon,
            Window::Roller,
        ] {
            let name = format!("WINDOW_{}", modern.name());
            assert!(
                !windows
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .any(|w| w == name),
                "{name}"
            );
        }
        for period in [
            Window::CarriageDoor,
            Window::Shoji,
            Window::Amado,
            Window::Punched,
            Window::Awning,
            Window::Signboard,
            Window::Louvres,
        ] {
            assert!(
                windows.contains(&format!("WINDOW_{}", period.name())),
                "{period:?}"
            );
        }
        // Panels and tile become render; the past's material and windows are taken only for a past chunk.
        let walls = function(shader, "wall_material_past");
        assert!(walls.contains(
            "if m == MATERIAL_PANELS || m == MATERIAL_TILE {\n        m = MATERIAL_RENDER;"
        ));
        assert!(!walls.contains("m = MATERIAL_TILE") && !walls.contains("m = MATERIAL_PANELS"));
        let textured = function(shader, "wall_textured");
        assert!(textured.contains("if in.past == 1u {\n        material = wall_material_past("));
        assert!(textured.contains("if in.past == 1u {\n            style = window_style_past("));
    }

    #[test]
    fn the_shader_names_the_layers_in_order() {
        let shader = include_str!("../../shaders/buildings.wgsl");
        for (k, w) in Window::ALL.iter().enumerate() {
            let line = format!("const WINDOW_{}: u32 = {k}u;", w.name());
            assert!(shader.contains(&line), "{line}");
        }
        for (k, m) in Material::ALL.iter().enumerate() {
            let line = format!("const MATERIAL_{}: u32 = {k}u;", m.name());
            assert!(shader.contains(&line), "{line}");
        }
        let count = format!("const WINDOW_COUNT: u32 = {}u;", Window::ALL.len());
        assert!(shader.contains(&count), "{count}");
    }

    #[test]
    fn a_windows_recess_is_its_frames_and_glass_not_its_trim() {
        let f = bake();
        let s = WINDOW_TEXELS;
        let layer = |w: Window| Window::ALL.iter().position(|x| *x == w).unwrap() as u32;
        let recess = |w: Window, u: f32, v: f32| {
            let (x, y) = ((u * s as f32) as u32, (v * s as f32) as u32);
            f.window_surface
                .texel(Window::ALL.len() as u32 + layer(w), x, y)[0]
        };
        // A sash's glass and frame lie back; its head, sill and the wall about it do not.
        assert_eq!(recess(Window::Sash, 0.4, 0.3), 255);
        assert_eq!(recess(Window::Sash, 0.305, 0.5), 255);
        assert_eq!(recess(Window::Sash, 0.5, 0.71), 0);
        assert_eq!(recess(Window::Sash, 0.05, 0.05), 0);
        // A punched window's stone surround stands at the wall, its reveal and sash behind it.
        assert_eq!(recess(Window::Punched, 0.28, 0.5), 0);
        assert_eq!(recess(Window::Punched, 0.5, 0.5), 255);
        // Shutters stand proud.
        assert_eq!(recess(Window::SashShuttered, 0.25, 0.4), 0);
        // Every window has some part set back.
        for w in Window::ALL {
            let n = (0..s)
                .flat_map(|y| (0..s).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    f.window_surface
                        .texel(Window::ALL.len() as u32 + layer(w), x, y)[0]
                        > 127
                })
                .count();
            assert!(n > 0, "{w:?}");
        }
    }
}
