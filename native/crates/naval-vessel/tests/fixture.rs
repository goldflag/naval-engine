//! Tiny ships made in code as GLBs (no model file is committed): a hull with a fine bow and a broad stern, a deckhouse
//! and a mast, built in ship space and written in whatever frame, units and origin a test asks for, with the
//! materials and textures it asks for.

use glam::Vec3;
use serde_json::{Value, json};

/// A model's surfaces in ship space (m: +X starboard, +Y up from the waterline, −Z ahead), by material.
pub struct Shape {
    pub parts: Vec<Part>,
}

pub struct Part {
    pub positions: Vec<Vec3>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub material: usize,
}

impl Part {
    /// A quad wound so its face points `outward`.
    fn quad(&mut self, mut corners: [Vec3; 4], outward: Vec3) {
        let [a, b, c, _] = corners;
        if (b - a).cross(c - a).dot(outward) < 0.0 {
            corners.reverse();
        }
        let base = self.positions.len() as u32;
        self.positions.extend(corners);
        self.uvs
            .extend([[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// A triangle wound so its face points `outward`.
    fn triangle(&mut self, mut corners: [Vec3; 3], outward: Vec3) {
        let [a, b, c] = corners;
        if (b - a).cross(c - a).dot(outward) < 0.0 {
            corners.swap(1, 2);
        }
        let base = self.positions.len() as u32;
        self.positions.extend(corners);
        self.uvs.extend([[0.5, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        self.indices.extend([base, base + 1, base + 2]);
    }

    /// A box's six faces, wound outward.
    pub fn cuboid(&mut self, min: Vec3, max: Vec3) {
        let v = Vec3::new;
        let (a, b) = (min, max);
        self.quad(
            [
                v(b.x, a.y, b.z),
                v(b.x, a.y, a.z),
                v(b.x, b.y, a.z),
                v(b.x, b.y, b.z),
            ],
            Vec3::X,
        );
        self.quad(
            [
                v(a.x, a.y, a.z),
                v(a.x, a.y, b.z),
                v(a.x, b.y, b.z),
                v(a.x, b.y, a.z),
            ],
            Vec3::NEG_X,
        );
        self.quad(
            [
                v(a.x, b.y, b.z),
                v(b.x, b.y, b.z),
                v(b.x, b.y, a.z),
                v(a.x, b.y, a.z),
            ],
            Vec3::Y,
        );
        self.quad(
            [
                v(a.x, a.y, a.z),
                v(b.x, a.y, a.z),
                v(b.x, a.y, b.z),
                v(a.x, a.y, b.z),
            ],
            Vec3::NEG_Y,
        );
        self.quad(
            [
                v(a.x, a.y, b.z),
                v(b.x, a.y, b.z),
                v(b.x, b.y, b.z),
                v(a.x, b.y, b.z),
            ],
            Vec3::Z,
        );
        self.quad(
            [
                v(b.x, a.y, a.z),
                v(a.x, a.y, a.z),
                v(a.x, b.y, a.z),
                v(b.x, b.y, a.z),
            ],
            Vec3::NEG_Z,
        );
    }

    fn new(material: usize) -> Self {
        Self {
            positions: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            material,
        }
    }
}

/// Her length (m), her beam, her keel's depth and her deck's height over the waterline.
pub const LENGTH: f32 = 100.0;
pub const BEAM: f32 = 20.0;
pub const DRAFT: f32 = 4.0;
pub const FREEBOARD: f32 = 4.0;
/// The deckhouse: across x −2..2, up from the deck by this, z −10..10.
pub const HOUSE: f32 = 6.0;

impl Shape {
    /// The ship: a hull whose sides run in to a point at the bow (z −50) over her fore 30 m and end square at the
    /// stern (z 50), decked over, a deckhouse amidships (material 1) and a mast on it (material 2, when asked).
    pub fn ship(mast: bool) -> Self {
        let mut hull = Part::new(0);
        let half = BEAM / 2.0;
        let (bow, shoulder, stern) = (-LENGTH / 2.0, -LENGTH / 2.0 + 30.0, LENGTH / 2.0);
        let (keel, deck) = (-DRAFT, FREEBOARD);
        let v = Vec3::new;
        // Starboard and port sides, the fore body's and the after body's.
        let fore = Vec3::new(1.0, 0.0, -0.3);
        hull.quad(
            [
                v(0.0, keel, bow),
                v(half, keel, shoulder),
                v(half, deck, shoulder),
                v(0.0, deck, bow),
            ],
            fore,
        );
        hull.quad(
            [
                v(half, keel, shoulder),
                v(half, keel, stern),
                v(half, deck, stern),
                v(half, deck, shoulder),
            ],
            Vec3::X,
        );
        hull.quad(
            [
                v(-half, keel, shoulder),
                v(0.0, keel, bow),
                v(0.0, deck, bow),
                v(-half, deck, shoulder),
            ],
            fore * Vec3::new(-1.0, 1.0, 1.0),
        );
        hull.quad(
            [
                v(-half, keel, stern),
                v(-half, keel, shoulder),
                v(-half, deck, shoulder),
                v(-half, deck, stern),
            ],
            Vec3::NEG_X,
        );
        // The transom, the deck and the bottom.
        hull.quad(
            [
                v(half, keel, stern),
                v(-half, keel, stern),
                v(-half, deck, stern),
                v(half, deck, stern),
            ],
            Vec3::Z,
        );
        hull.quad(
            [
                v(-half, deck, shoulder),
                v(half, deck, shoulder),
                v(half, deck, stern),
                v(-half, deck, stern),
            ],
            Vec3::Y,
        );
        hull.triangle(
            [
                v(0.0, deck, bow),
                v(half, deck, shoulder),
                v(-half, deck, shoulder),
            ],
            Vec3::Y,
        );
        hull.quad(
            [
                v(-half, keel, stern),
                v(half, keel, stern),
                v(half, keel, shoulder),
                v(-half, keel, shoulder),
            ],
            Vec3::NEG_Y,
        );
        hull.triangle(
            [
                v(0.0, keel, bow),
                v(-half, keel, shoulder),
                v(half, keel, shoulder),
            ],
            Vec3::NEG_Y,
        );
        let mut house = Part::new(1);
        house.cuboid(v(-2.0, deck, -10.0), v(2.0, deck + HOUSE, 10.0));
        let mut parts = vec![hull, house];
        if mast {
            let mut spar = Part::new(2);
            spar.cuboid(
                v(-0.3, deck + HOUSE, -4.0),
                v(0.3, deck + HOUSE + 16.0, -3.4),
            );
            parts.push(spar);
        }
        Self { parts }
    }
}

/// A GLB written from a [`Shape`]: its positions taken through `frame` (ship space to the file's coordinates).
pub struct Glb {
    pub json: Value,
    pub bin: Vec<u8>,
}

fn pad(bytes: &mut Vec<u8>, with: u8) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(with);
    }
}

impl Glb {
    /// `shape` in the file's frame, with `materials` (glTF material JSON, by the parts' material indices) and, when
    /// `uvs`, a TEXCOORD_0 (normals are left to the reader: flat).
    pub fn new(
        shape: &Shape,
        frame: impl Fn(Vec3) -> Vec3,
        materials: Vec<Value>,
        uvs: bool,
    ) -> Self {
        let mut glb = Self {
            json: json!({
                "asset": {"version": "2.0", "generator": "naval-vessel tests"},
                "scene": 0,
                "scenes": [{"nodes": [0]}],
                "nodes": [{"mesh": 0}],
                "meshes": [{"primitives": []}],
                "materials": materials,
                "buffers": [{"byteLength": 0}],
                "bufferViews": [],
                "accessors": [],
            }),
            bin: Vec::new(),
        };
        for part in &shape.parts {
            let points: Vec<Vec3> = part.positions.iter().map(|&p| frame(p)).collect();
            let (min, max) = points.iter().fold(
                (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                |(lo, hi), &p| (lo.min(p), hi.max(p)),
            );
            let floats: Vec<f32> = points.iter().flat_map(|p| p.to_array()).collect();
            let position = glb.accessor(
                bytemuck::cast_slice(&floats),
                5126,
                "VEC3",
                points.len(),
                Some((min, max)),
                34962,
            );
            let mut attributes = json!({"POSITION": position});
            if uvs {
                let flat: Vec<f32> = part.uvs.iter().flatten().copied().collect();
                attributes["TEXCOORD_0"] = json!(glb.accessor(
                    bytemuck::cast_slice(&flat),
                    5126,
                    "VEC2",
                    part.uvs.len(),
                    None,
                    34962
                ));
            }
            let indices = glb.accessor(
                bytemuck::cast_slice(&part.indices),
                5125,
                "SCALAR",
                part.indices.len(),
                None,
                34963,
            );
            glb.json["meshes"][0]["primitives"]
                .as_array_mut()
                .unwrap()
                .push(json!({"attributes": attributes, "indices": indices, "material": part.material}));
        }
        glb
    }

    /// Bytes as a buffer view, and an accessor of them; returns the accessor's index.
    fn accessor(
        &mut self,
        bytes: &[u8],
        component: u32,
        kind: &str,
        count: usize,
        bounds: Option<(Vec3, Vec3)>,
        target: u32,
    ) -> usize {
        let view = self.view(bytes, Some(target));
        let accessors = self.json["accessors"].as_array_mut().unwrap();
        let mut accessor =
            json!({"bufferView": view, "componentType": component, "count": count, "type": kind});
        if let Some((min, max)) = bounds {
            accessor["min"] = json!(min.to_array());
            accessor["max"] = json!(max.to_array());
        }
        accessors.push(accessor);
        accessors.len() - 1
    }

    /// Bytes as a buffer view; returns its index.
    pub fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        let offset = self.bin.len();
        self.bin.extend_from_slice(bytes);
        pad(&mut self.bin, 0);
        let views = self.json["bufferViews"].as_array_mut().unwrap();
        let mut view = json!({"buffer": 0, "byteOffset": offset, "byteLength": bytes.len()});
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        views.push(view);
        views.len() - 1
    }

    /// A PNG of `width` × `height` RGBA texels as image and texture; returns the texture's index.
    pub fn texture(
        &mut self,
        width: u32,
        height: u32,
        texel: impl Fn(u32, u32) -> [u8; 4],
    ) -> usize {
        let image = image::RgbaImage::from_fn(width, height, |x, y| image::Rgba(texel(x, y)));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let view = self.view(&png, None);
        let images = self.json["images"].as_array().map_or(0, Vec::len);
        let textures = self.json["textures"].as_array().map_or(0, Vec::len);
        if images == 0 {
            self.json["images"] = json!([]);
            self.json["textures"] = json!([]);
            self.json["samplers"] = json!([{}]);
        }
        self.json["images"].as_array_mut().unwrap().push(
            json!({"bufferView": view, "mimeType": "image/png", "name": format!("map {images}")}),
        );
        self.json["textures"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source": images, "sampler": 0}));
        textures
    }

    /// The GLB's bytes.
    pub fn bytes(&self) -> Vec<u8> {
        let mut json = serde_json::to_vec(&{
            let mut j = self.json.clone();
            j["buffers"][0]["byteLength"] = json!(self.bin.len());
            j
        })
        .unwrap();
        pad(&mut json, b' ');
        let total = 12 + 8 + json.len() + 8 + self.bin.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        out.extend_from_slice(&(self.bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&self.bin);
        out
    }
}

/// A plain painted material: grey steel, or a colour.
pub fn paint(name: &str, rgb: [f32; 3], roughness: f32) -> Value {
    json!({
        "name": name,
        "pbrMetallicRoughness": {"baseColorFactor": [rgb[0], rgb[1], rgb[2], 1.0], "metallicFactor": 0.0, "roughnessFactor": roughness},
    })
}

/// The ship as glTF has her (+Y up, −Z ahead, metres, her origin on the waterline amidships): grey hull, white
/// deckhouse, black mast.
pub fn plain() -> Glb {
    Glb::new(
        &Shape::ship(true),
        |p| p,
        vec![
            paint("hull", [0.35, 0.37, 0.4], 0.6),
            paint("house", [0.8, 0.8, 0.78], 0.6),
            paint("mast", [0.05, 0.05, 0.05], 0.7),
        ],
        false,
    )
}
