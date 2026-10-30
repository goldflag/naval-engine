//! The setup file beside a model (`<model>.vessel.json`): which model, how big, which way is up and ahead, where the
//! sea stands on her, and her funnels and lamps. Every downloaded model needs one, since nothing in a glTF says how
//! long a ship is or where her waterline lies; `naval-vessel inspect <model>` writes a starter. [`crate::Vessel`] is
//! what it resolves to.
//!
//! Points (`at`) are in ship space, metres: +X starboard, +Y up from the waterline, −Z ahead, the origin on the
//! waterline amidships (half way between the model's bow and stern, on its centreline).

use anyhow::{Context, Result, bail, ensure};
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// The setup file's version.
pub const SETUP_VERSION: u32 = 1;

/// Textures larger than this on a side are halved until they fit (Sketchfab's are often 4096 or 8192).
pub const DEFAULT_TEXTURE_SIZE: u32 = 2048;

/// One of the model's axes, signed: the way its up or its bow points in the file's own coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Axis {
    /// 0 x, 1 y, 2 z.
    pub index: usize,
    pub negative: bool,
}

impl Axis {
    pub const X: Axis = Axis::new(0, false);
    pub const Y: Axis = Axis::new(1, false);
    pub const Z: Axis = Axis::new(2, false);
    pub const NEG_X: Axis = Axis::new(0, true);
    pub const NEG_Y: Axis = Axis::new(1, true);
    pub const NEG_Z: Axis = Axis::new(2, true);

    pub const fn new(index: usize, negative: bool) -> Self {
        Self { index, negative }
    }

    /// The unit vector it points along.
    pub fn vector(self) -> Vec3 {
        let mut v = Vec3::ZERO;
        v[self.index] = if self.negative { -1.0 } else { 1.0 };
        v
    }

    pub fn sign(self) -> f32 {
        if self.negative { -1.0 } else { 1.0 }
    }

    pub fn letter(self) -> char {
        ['x', 'y', 'z'][self.index]
    }
}

impl std::fmt::Display for Axis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}{}",
            if self.negative { '-' } else { '+' },
            self.letter()
        )
    }
}

impl TryFrom<String> for Axis {
    type Error = String;

    fn try_from(value: String) -> Result<Self, String> {
        let s = value.trim().to_ascii_lowercase();
        let (negative, letter) = match s.as_bytes() {
            [b'-', l] => (true, *l),
            [b'+', l] | [l] => (false, *l),
            _ => {
                return Err(format!(
                    "an axis is +x, -x, +y, -y, +z or -z, not {value:?}"
                ));
            }
        };
        let index = match letter {
            b'x' => 0,
            b'y' => 1,
            b'z' => 2,
            _ => {
                return Err(format!(
                    "an axis is +x, -x, +y, -y, +z or -z, not {value:?}"
                ));
            }
        };
        Ok(Axis::new(index, negative))
    }
}

impl From<Axis> for String {
    fn from(axis: Axis) -> String {
        axis.to_string()
    }
}

/// What a lamp is, which sets its arc and its colour as the rules of the road have them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LampKind {
    /// White, over 225° ahead (right ahead to 22.5° abaft either beam).
    Masthead,
    /// Over 112.5° on its side, from right ahead: red to port, green to starboard (by the side of the centreline it
    /// stands on).
    Side,
    /// White, over 135° astern.
    Stern,
    /// White, all round (an anchor light, or any all-round lamp).
    Anchor,
}

impl LampKind {
    /// Its luminous intensity (candela) for a ship over 50 m: a masthead light seen 6 miles, the others 3 (COLREGs
    /// Annex I, 8: I = 3.43 × 10⁶ × T × D² × K^−D, T 2 × 10⁻⁷ lux, K 0.8).
    pub fn intensity(self) -> f32 {
        match self {
            LampKind::Masthead => 94.0,
            LampKind::Side | LampKind::Stern | LampKind::Anchor => 12.0,
        }
    }
}

/// A funnel in the setup file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunnelSetup {
    #[serde(default)]
    pub name: String,
    /// The middle of its mouth, ship space (m).
    pub at: [f32; 3],
    /// How much smoke it gives, 0 none to 1 a ship steaming hard (more is allowed).
    #[serde(default = "one")]
    pub smoke: f32,
    /// Its mouth's radius (m), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
}

/// A lamp in the setup file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LampSetup {
    #[serde(default)]
    pub name: String,
    /// Ship space (m).
    pub at: [f32; 3],
    pub kind: LampKind,
    /// Linear RGB; the kind's own colour when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    /// Candela; the kind's own ([`LampKind::intensity`]) when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f32>,
}

/// A model's setup file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Setup {
    #[serde(default = "version")]
    pub version: u32,
    /// The model, relative to the setup file: a `.glb`, or a `.gltf` with its buffers and images beside it.
    pub model: String,
    /// Her length overall (m): the model is scaled so it is this long bow to stern. Give this or `scale`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f32>,
    /// Metres per model unit, when the model's own size is known (1 for a model in metres, 0.01 in centimetres).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    /// The model's axis that points up (+y, glTF's own, by default).
    #[serde(default = "up")]
    pub up: Axis,
    /// The model's axis that points from stern to bow.
    #[serde(default = "forward")]
    pub forward: Axis,
    /// Where the sea stands on her: the model's own coordinate on its `up` axis's letter at the waterline (model
    /// units, as the file has it: for `up` "-z", its z there). Give this or `draft`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waterline: Option<f32>,
    /// Her draft (m): from the waterline down to the model's lowest point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<f32>,
    /// Her beam (m), when the model's own breadth is not it (boats swung out, yards, a crane).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beam: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub funnels: Vec<FunnelSetup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lights: Vec<LampSetup>,
    /// The largest side a texture keeps (texels).
    #[serde(default = "texture_size")]
    pub texture_size: u32,
}

fn version() -> u32 {
    SETUP_VERSION
}

fn one() -> f32 {
    1.0
}

fn up() -> Axis {
    Axis::Y
}

fn forward() -> Axis {
    Axis::NEG_Z
}

fn texture_size() -> u32 {
    DEFAULT_TEXTURE_SIZE
}

impl Setup {
    /// A setup for `model` with every default: glTF's axes (+Y up, −Z ahead), metres.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            version: SETUP_VERSION,
            model: model.into(),
            length: None,
            scale: None,
            up: up(),
            forward: forward(),
            waterline: None,
            draft: None,
            beam: None,
            funnels: Vec::new(),
            lights: Vec::new(),
            texture_size: texture_size(),
        }
    }

    /// Reads and checks a setup file.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let setup: Setup = serde_json::from_slice(bytes).context("reading the setup file")?;
        setup.check()?;
        Ok(setup)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a setup serialises")
    }

    /// The rules a setup keeps: one way to size her, one to place her waterline, axes that are not parallel, finite
    /// points.
    pub fn check(&self) -> Result<()> {
        ensure!(
            self.version == SETUP_VERSION,
            "setup version {} (this build reads {SETUP_VERSION})",
            self.version
        );
        ensure!(!self.model.is_empty(), "the setup names no model");
        if self.length.is_some() && self.scale.is_some() {
            bail!("give `length` or `scale`, not both");
        }
        if self.waterline.is_some() && self.draft.is_some() {
            bail!("give `waterline` or `draft`, not both");
        }
        for (name, value) in [
            ("length", self.length),
            ("scale", self.scale),
            ("beam", self.beam),
        ] {
            if let Some(v) = value {
                ensure!(
                    v.is_finite() && v > 0.0,
                    "`{name}` must be above 0, not {v}"
                );
            }
        }
        if let Some(draft) = self.draft {
            ensure!(draft.is_finite(), "`draft` must be a number");
        }
        if let Some(waterline) = self.waterline {
            ensure!(waterline.is_finite(), "`waterline` must be a number");
        }
        ensure!(
            self.up.index != self.forward.index,
            "`up` ({}) and `forward` ({}) lie along one axis",
            self.up,
            self.forward
        );
        ensure!(self.texture_size >= 1, "`textureSize` must be at least 1");
        for funnel in &self.funnels {
            ensure!(
                funnel.at.iter().all(|v| v.is_finite()) && funnel.smoke.is_finite(),
                "funnel {:?}: its point and smoke must be numbers",
                funnel.name
            );
        }
        for lamp in &self.lights {
            ensure!(
                lamp.at.iter().all(|v| v.is_finite()),
                "lamp {:?}: its point must be numbers",
                lamp.name
            );
        }
        Ok(())
    }

    /// The rotation from the model's axes to ship space (+X starboard, +Y up, −Z ahead): its rows are the model's
    /// starboard, up and aft. Always a rotation: starboard is ahead × up in a right-handed file.
    pub fn rotation(&self) -> glam::Mat3 {
        let up = self.up.vector();
        let ahead = self.forward.vector();
        let starboard = ahead.cross(up);
        glam::Mat3::from_cols(starboard, up, -ahead).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_read_and_write_as_signed_letters() {
        for (text, axis) in [
            ("+x", Axis::X),
            ("-z", Axis::NEG_Z),
            ("y", Axis::Y),
            (" -Y ", Axis::NEG_Y),
        ] {
            assert_eq!(Axis::try_from(text.to_owned()).unwrap(), axis);
        }
        assert!(Axis::try_from("w".to_owned()).is_err());
        assert!(Axis::try_from("+-x".to_owned()).is_err());
        assert_eq!(String::from(Axis::NEG_Z), "-z");
    }

    #[test]
    fn the_rotation_takes_the_models_axes_to_ship_space() {
        // glTF's own: nothing turns.
        let gltf = Setup::new("a.glb");
        assert!(gltf.rotation().abs_diff_eq(glam::Mat3::IDENTITY, 1e-6));
        // Z up, bow along +X (a Blender model exported as is).
        let blender = Setup {
            up: Axis::Z,
            forward: Axis::X,
            ..Setup::new("b.glb")
        };
        let r = blender.rotation();
        assert!((r * Vec3::Z).abs_diff_eq(Vec3::Y, 1e-6));
        assert!((r * Vec3::X).abs_diff_eq(Vec3::NEG_Z, 1e-6));
        // Starboard: the model's −Y (ahead +X, up +Z: X × Z = −Y).
        assert!((r * Vec3::NEG_Y).abs_diff_eq(Vec3::X, 1e-6));
        assert!((r.determinant() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_setup_refuses_two_ways_to_size_her() {
        let both = r#"{"model": "a.glb", "length": 100, "scale": 0.01}"#;
        assert!(Setup::from_json(both.as_bytes()).is_err());
        let parallel = r#"{"model": "a.glb", "up": "+z", "forward": "-z"}"#;
        assert!(Setup::from_json(parallel.as_bytes()).is_err());
        let unknown = r#"{"model": "a.glb", "lenght": 100}"#;
        assert!(Setup::from_json(unknown.as_bytes()).is_err());
        let fine = r#"{"model": "a.glb", "length": 100, "draft": 3.2,
            "funnels": [{"name": "fore", "at": [0, 12, -8], "smoke": 0.8}],
            "lights": [{"name": "masthead", "at": [0, 30, -20], "kind": "masthead"}]}"#;
        let setup = Setup::from_json(fine.as_bytes()).unwrap();
        assert_eq!(setup.lights[0].kind, LampKind::Masthead);
        assert_eq!(Setup::from_json(setup.to_json().as_bytes()).unwrap(), setup);
    }
}
