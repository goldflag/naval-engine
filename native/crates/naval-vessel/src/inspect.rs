//! What a downloaded model is before it has a setup ([`Inspection`]): its bounds, a guess at its axes, its materials,
//! triangles and textures, and a starter setup file from the guess. `naval-vessel inspect <model>` prints it.

use std::fmt::Write as _;

use anyhow::Result;
use glam::Vec3;

use crate::images;
use crate::read::{self, Alpha, Resolver, Slot};
use crate::setup::{Axis, Setup};
use crate::vessel::{Bounds, GUESSED_DRAFT};

/// Which way the model's up and bow seem to point, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct AxesGuess {
    pub up: Axis,
    pub forward: Axis,
    /// One line each for up and the bow.
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterialSummary {
    pub name: String,
    pub alpha: Alpha,
    pub double_sided: bool,
    pub unlit: bool,
    pub glass: bool,
    /// Its maps: the slot and the file's image.
    pub maps: Vec<(Slot, usize)>,
    pub triangles: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageSummary {
    pub name: String,
    /// PNG, JPEG, WebP… by its first bytes, or why it could not be read.
    pub format: String,
    pub size: Option<(u32, u32)>,
    /// The maps it is.
    pub uses: Vec<Slot>,
}

/// A model looked over.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspection {
    pub name: String,
    pub binary: bool,
    pub generator: Option<String>,
    pub extensions_used: Vec<String>,
    pub extensions_required: Vec<String>,
    pub nodes: usize,
    pub meshes: usize,
    pub primitives: usize,
    pub triangles: usize,
    pub vertices: usize,
    /// In the model's own coordinates (its nodes' transforms applied), model units.
    pub bounds: Bounds,
    pub guess: AxesGuess,
    pub materials: Vec<MaterialSummary>,
    pub images: Vec<ImageSummary>,
    pub notes: Vec<String>,
}

/// Looks over `bytes` (a GLB, or a `.gltf` whose other files `resolver` reads). A file that would be refused is
/// refused here too, with the reason.
pub fn inspect(name: &str, bytes: &[u8], resolver: Resolver<'_>) -> Result<Inspection> {
    read::guarded(|| look_over(name, bytes, resolver))
}

fn look_over(name: &str, bytes: &[u8], resolver: Resolver<'_>) -> Result<Inspection> {
    let file = read::parse(bytes, &mut *resolver)?;
    let mesh = read::scene_mesh(&file)?;
    let bounds = Bounds::of(mesh.positions.iter().copied());
    let guess = guess_axes(&mesh.positions, &mesh.groups, bounds);
    let read_materials = read::materials(&file);
    read::check_uv_sets(&read_materials, &mesh)?;
    let mut materials: Vec<MaterialSummary> = read_materials
        .into_iter()
        .map(|m| MaterialSummary {
            maps: Slot::ALL
                .iter()
                .filter_map(|&s| m.maps[s as usize].map(|map| (s, map.texture)))
                .collect(),
            name: m.name,
            alpha: m.alpha,
            double_sided: m.double_sided,
            unlit: m.unlit,
            glass: m.transmission > 0.0,
            triangles: 0,
        })
        .collect();
    for (material, triangles) in &mesh.groups {
        let Some(i) = material else { continue };
        if let Some(summary) = materials.get_mut(*i) {
            summary.triangles += triangles.len() / 3;
        }
    }
    let images = (0..file.document.images().len())
        .map(|i| {
            let uses = materials
                .iter()
                .flat_map(|m| m.maps.iter())
                .filter(|(_, image)| *image == i)
                .map(|(slot, _)| *slot)
                .fold(Vec::new(), |mut v, s| {
                    if !v.contains(&s) {
                        v.push(s);
                    }
                    v
                });
            match read::image_bytes(&file, i, &mut *resolver) {
                Ok((bytes, name)) => ImageSummary {
                    name,
                    format: images::sniff(&bytes).to_owned(),
                    size: images::dimensions(&bytes),
                    uses,
                },
                Err(e) => ImageSummary {
                    name: format!("image {i}"),
                    format: format!("unreadable: {e:#}"),
                    size: None,
                    uses,
                },
            }
        })
        .collect();
    let json = &file.json;
    Ok(Inspection {
        name: name.to_owned(),
        binary: file.binary,
        generator: json["asset"]["generator"].as_str().map(str::to_owned),
        extensions_used: read::strings(&json["extensionsUsed"])
            .into_iter()
            .map(str::to_owned)
            .collect(),
        extensions_required: read::strings(&json["extensionsRequired"])
            .into_iter()
            .map(str::to_owned)
            .collect(),
        nodes: file.document.nodes().len(),
        meshes: file.document.meshes().len(),
        primitives: mesh.primitives,
        triangles: mesh.triangles(),
        vertices: mesh.positions.len(),
        bounds,
        guess,
        materials,
        images,
        notes: mesh.notes,
    })
}

/// The bow and the up of a ship: she is longest bow to stern; up is the taller of the other two (masts stand higher
/// than she is broad), glTF's +Y when they are near; up points the way her bounds reach farther from the middle of her
/// surfaces (masts and upperworks are thin, the hull below them is not); her bow is the finer end, measured near each
/// end.
pub fn guess_axes(
    positions: &[Vec3],
    groups: &[(Option<usize>, Vec<u32>)],
    bounds: Bounds,
) -> AxesGuess {
    let size = bounds.size();
    let mut reasons = Vec::new();
    let long = (0..3)
        .max_by(|&a, &b| size[a].total_cmp(&size[b]))
        .unwrap_or(2);
    let others: Vec<usize> = (0..3).filter(|&i| i != long).collect();
    let (a, b) = (others[0], others[1]);
    let up_index = if others.contains(&1) && size[1] >= 0.85 * size[a].max(size[b]) {
        1
    } else if size[a] >= size[b] {
        a
    } else {
        b
    };
    let across = if up_index == a { b } else { a };
    // The surfaces' middle, by area.
    let mut centroid = Vec3::ZERO;
    let mut area = 0.0;
    for (_, triangles) in groups {
        for t in triangles.as_chunks::<3>().0 {
            let [p, q, r] = [0, 1, 2].map(|k| positions[t[k] as usize]);
            let w = (q - p).cross(r - p).length() * 0.5;
            centroid += (p + q + r) / 3.0 * w;
            area += w;
        }
    }
    let centroid = if area > 0.0 {
        centroid / area
    } else {
        bounds.centre()
    };
    let lean = bounds.centre()[up_index] - centroid[up_index];
    let up_negative = lean < -0.02 * size[up_index];
    let up = Axis::new(up_index, up_negative);
    reasons.push(format!(
        "up {up}: {} is her taller side across her length ({:.1} against {:.1}){}",
        up.letter(),
        size[up_index],
        size[across],
        if lean.abs() < 0.02 * size[up_index] {
            ", its sign glTF's (her surfaces sit even about her middle)".to_owned()
        } else {
            format!(
                ", and her surfaces sit {:.0} % of her height {} her middle",
                lean.abs() / size[up_index] * 100.0,
                if up_negative { "above" } else { "below" }
            )
        }
    ));
    // Her breadth near each end: where her surfaces cross a station 5 % of her length in from it.
    let middle = bounds.centre()[across];
    let breadth = |station: f32| {
        let mut widest = 0f32;
        for (_, triangles) in groups {
            for t in triangles.as_chunks::<3>().0 {
                for (i, j) in [(0, 1), (1, 2), (2, 0)] {
                    let (p, q) = (positions[t[i] as usize], positions[t[j] as usize]);
                    if (p[long] < station) != (q[long] < station) {
                        let at = p.lerp(q, (station - p[long]) / (q[long] - p[long]));
                        widest = widest.max((at[across] - middle).abs());
                    }
                }
            }
        }
        widest
    };
    let (at_max, at_min) = (
        breadth(bounds.max[long] - 0.05 * size[long]),
        breadth(bounds.min[long] + 0.05 * size[long]),
    );
    let forward = Axis::new(long, at_min < at_max);
    let (fine, full) = (at_max.min(at_min), at_max.max(at_min));
    reasons.push(if full > 0.0 && fine / full < 0.9 {
        format!(
            "bow {forward}: her {} end is the finer ({:.0} % of the other's breadth near it)",
            if forward.negative { "-" } else { "+" },
            fine / full * 100.0
        )
    } else {
        format!("bow {forward}: unsure, her ends are about as fine: check it (the bow may be the other end)")
    });
    AxesGuess {
        up,
        forward,
        reasons,
    }
}

impl Inspection {
    /// Her length along the guessed bow, model units.
    pub fn length(&self) -> f32 {
        self.bounds.size()[self.guess.forward.index]
    }

    /// The length in metres the model's size suggests: its units as metres, or centimetres or millimetres when that
    /// makes a ship over 500 m.
    pub fn guessed_scale(&self) -> (f32, &'static str) {
        let length = self.length();
        if length > 50_000.0 {
            (0.001, "millimetres")
        } else if length > 500.0 {
            (0.01, "centimetres")
        } else {
            (1.0, "metres")
        }
    }

    /// A setup from the guess, for `model` (its path relative to the setup file): her length from the model's size
    /// as guessed, her draft a guess, no funnels or lamps yet.
    pub fn starter(&self, model: &str) -> Setup {
        let (scale, _) = self.guessed_scale();
        let length = (self.length() * scale * 10.0).round() / 10.0;
        Setup {
            length: Some(length),
            up: self.guess.up,
            forward: self.guess.forward,
            draft: Some((length * GUESSED_DRAFT * 10.0).round() / 10.0),
            ..Setup::new(model)
        }
    }

    /// The starter as JSON, its empty lists of funnels and lamps written out to fill in.
    pub fn starter_json(&self, model: &str) -> String {
        let mut value = serde_json::to_value(self.starter(model)).expect("a setup serialises");
        value["funnels"] = serde_json::json!([]);
        value["lights"] = serde_json::json!([]);
        serde_json::to_string_pretty(&value).expect("JSON serialises")
    }

    /// The report `naval-vessel inspect` prints.
    pub fn report(&self, texture_size: u32) -> String {
        let mut out = String::new();
        let o = &mut out;
        let kind = if self.binary { "GLB" } else { "glTF JSON" };
        let _ = writeln!(
            o,
            "{} ({kind}{})",
            self.name,
            self.generator
                .as_deref()
                .map(|g| format!(", made by {g}"))
                .unwrap_or_default()
        );
        let drawn: Vec<&str> = self
            .extensions_used
            .iter()
            .map(String::as_str)
            .filter(|e| read::DRAWN_EXTENSIONS.contains(e))
            .collect();
        let ignored: Vec<&str> = self
            .extensions_used
            .iter()
            .map(String::as_str)
            .filter(|e| !read::DRAWN_EXTENSIONS.contains(e))
            .collect();
        let list = |v: &[&str]| {
            if v.is_empty() {
                "none".to_owned()
            } else {
                v.join(", ")
            }
        };
        let _ = writeln!(
            o,
            "  extensions drawn: {}; left out: {}",
            list(&drawn),
            list(&ignored)
        );
        let _ = writeln!(
            o,
            "  {} nodes, {} meshes, {} primitives: {} triangles, {} vertices",
            self.nodes, self.meshes, self.primitives, self.triangles, self.vertices
        );
        let b = &self.bounds;
        let s = b.size();
        let _ = writeln!(o, "  bounds (model units, its nodes' transforms applied):");
        for (k, letter) in ['x', 'y', 'z'].into_iter().enumerate() {
            let _ = writeln!(
                o,
                "    {letter} {:>10.3} .. {:>10.3}  ({:.3})",
                b.min[k], b.max[k], s[k]
            );
        }
        let _ = writeln!(
            o,
            "  axes guess: up {}, forward {}",
            self.guess.up, self.guess.forward
        );
        for reason in &self.guess.reasons {
            let _ = writeln!(o, "    {reason}");
        }
        let (scale, units) = self.guessed_scale();
        let _ = writeln!(
            o,
            "  length {:.3} units: {:.1} m if in {units} (set `length` to her real length)",
            self.length(),
            self.length() * scale
        );
        let _ = writeln!(o, "  materials ({}):", self.materials.len());
        for (i, m) in self.materials.iter().enumerate() {
            let alpha = match m.alpha {
                Alpha::Opaque if m.glass => "glass".to_owned(),
                Alpha::Opaque => "opaque".to_owned(),
                Alpha::Mask(c) => format!("cut-out at {c}"),
                Alpha::Blend if m.glass => "glass".to_owned(),
                Alpha::Blend => "blended".to_owned(),
            };
            let maps: Vec<String> = m
                .maps
                .iter()
                .map(|(s, image)| format!("{} {image}", s.name()))
                .collect();
            let _ = writeln!(
                o,
                "    {i} {:?}: {alpha}{}{}, {} triangles; maps: {}",
                m.name,
                if m.double_sided { ", two-sided" } else { "" },
                if m.unlit { ", unlit" } else { "" },
                m.triangles,
                if maps.is_empty() {
                    "none".to_owned()
                } else {
                    maps.join(", ")
                }
            );
        }
        let _ = writeln!(
            o,
            "  images ({}), each kept at most {texture_size} on a side:",
            self.images.len()
        );
        for (i, image) in self.images.iter().enumerate() {
            let size = image
                .size
                .map(|(w, h)| {
                    let (mut dw, mut dh) = (w, h);
                    while dw.max(dh) > texture_size.max(1) {
                        dw = (dw / 2).max(1);
                        dh = (dh / 2).max(1);
                    }
                    if (dw, dh) == (w, h) {
                        format!("{w} x {h}")
                    } else {
                        format!("{w} x {h}, drawn {dw} x {dh}")
                    }
                })
                .unwrap_or_else(|| "size unknown".into());
            let uses: Vec<&str> = image.uses.iter().map(|s| s.name()).collect();
            let _ = writeln!(
                o,
                "    {i} {:?}: {}, {size}; {}",
                image.name,
                image.format,
                if uses.is_empty() {
                    "unused".to_owned()
                } else {
                    uses.join(", ")
                }
            );
        }
        for note in &self.notes {
            let _ = writeln!(o, "  note: {note}");
        }
        out
    }
}
