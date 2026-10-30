//! Reading a glTF 2.0 file: its JSON (`.gltf`) or GLB, its buffers, its scene's triangles in the scene's own
//! coordinates and its materials. What this crate cannot draw is refused here, by name, before anything is read: a
//! compressed mesh, a compressed or WebP texture, a required extension it does not know. Nothing a file holds makes it
//! panic or hang: references are checked by `gltf`'s validation (what it reads unchecked, before it), each accessor's
//! kind and every byte `gltf`'s readers will touch here before they read (they index unchecked, assert and hit
//! `unreachable!` on what they do not expect), counts are bounded before anything is allocated for them, and the scene
//! is walked as the forest glTF says it is.

use std::borrow::Cow;

use anyhow::{Context, Result, bail, ensure};
use glam::{Mat3, Mat4, Vec3};
use gltf::accessor::{DataType, Dimensions};
use gltf::mesh::Mode;
use serde_json::Value;

/// Hands the bytes of a file a `.gltf` names (its buffers and images, by their URIs decoded, relative to it).
pub type Resolver<'a> = &'a mut dyn FnMut(&str) -> Result<Vec<u8>>;

/// The most vertices a model may draw (44 bytes each: wgpu's default largest buffer is 256 MiB).
pub const MAX_VERTICES: usize = 5_000_000;
/// The most triangles.
pub const MAX_TRIANGLES: usize = 10_000_000;

/// Extensions a file may require that this crate reads.
pub const DRAWN_EXTENSIONS: &[&str] = &[
    "KHR_texture_transform",
    "KHR_materials_emissive_strength",
    "KHR_materials_unlit",
    "KHR_materials_pbrSpecularGlossiness",
    "KHR_materials_transmission",
];

/// Extensions refused when a file requires them, and why.
pub const REFUSED_EXTENSIONS: &[(&str, &str)] = &[
    ("KHR_draco_mesh_compression", "Draco-compressed meshes"),
    (
        "EXT_meshopt_compression",
        "meshoptimizer-compressed buffers",
    ),
    ("KHR_mesh_quantization", "quantized vertex attributes"),
    ("KHR_texture_basisu", "KTX2 (Basis Universal) textures"),
    ("EXT_texture_webp", "WebP textures"),
    ("EXT_texture_avif", "AVIF textures"),
    ("EXT_mesh_gpu_instancing", "GPU-instanced meshes"),
];

/// How to make a refused file drawable.
const REEXPORT: &str = "re-export it without compression and with PNG or JPEG images (Blender: File > Export > \
                        glTF 2.0, Compression off, Images Automatic)";

/// A parsed file: its document, its buffers' bytes and its raw JSON (for the extensions `gltf` is built without).
pub(crate) struct File {
    pub document: gltf::Document,
    pub buffers: Vec<Vec<u8>>,
    pub json: Value,
    pub binary: bool,
}

/// Parses a GLB or a `.gltf`'s JSON, refusing what cannot be drawn, and loads its buffers.
pub(crate) fn parse(bytes: &[u8], resolver: Resolver<'_>) -> Result<File> {
    let binary = bytes.starts_with(b"glTF");
    let (json_bytes, mut blob): (Cow<'_, [u8]>, Option<Vec<u8>>) = if binary {
        let glb =
            gltf::Glb::from_slice(bytes).map_err(|e| anyhow::anyhow!("not a readable GLB: {e}"))?;
        (
            Cow::Owned(glb.json.into_owned()),
            glb.bin.map(Cow::into_owned),
        )
    } else {
        (Cow::Borrowed(bytes), None)
    };
    let json: Value =
        serde_json::from_slice(&json_bytes).context("not glTF: its JSON does not parse")?;
    refuse(&json)?;
    let root: gltf::json::Root = serde_json::from_value(json.clone())
        .context("not glTF 2.0: its JSON does not match the schema")?;
    preflight(&root)?;
    validate(&root)?;
    let document = gltf::Document::from_json_without_validation(root);
    let mut buffers = Vec::new();
    for buffer in document.buffers() {
        let data = match buffer.source() {
            // The binary chunk is buffer 0's alone, handed over once: more buffers with no URI would each copy it.
            gltf::buffer::Source::Bin => {
                ensure!(
                    buffer.index() == 0,
                    "buffer {} has no URI: only buffer 0 may be the GLB's binary chunk",
                    buffer.index()
                );
                blob.take()
                    .context("buffer 0 is the GLB's binary chunk, which it does not have")?
            }
            gltf::buffer::Source::Uri(uri) => {
                fetch(uri, resolver).with_context(|| format!("buffer {}", buffer.index()))?
            }
        };
        ensure!(
            data.len() >= buffer.length(),
            "buffer {} holds {} bytes of the {} it says",
            buffer.index(),
            data.len(),
            buffer.length()
        );
        buffers.push(data);
    }
    Ok(File {
        document,
        buffers,
        json,
        binary,
    })
}

/// Refuses glTF 1.0, the extensions in [`REFUSED_EXTENSIONS`] and any other required extension this crate does not
/// read, and textures with no PNG or JPEG source.
fn refuse(json: &Value) -> Result<()> {
    let version = json["asset"]["version"].as_str().unwrap_or("");
    ensure!(
        version.starts_with('2'),
        "glTF {version:?}: only glTF 2.0 is read"
    );
    for name in strings(&json["extensionsRequired"]) {
        if let Some((_, what)) = REFUSED_EXTENSIONS.iter().find(|(ext, _)| *ext == name) {
            bail!("{what} ({name}) are not supported: {REEXPORT}");
        }
        ensure!(
            DRAWN_EXTENSIONS.contains(&name),
            "it requires {name}, which this crate does not read: {REEXPORT}"
        );
    }
    if let Some(textures) = json["textures"].as_array() {
        for (i, texture) in textures.iter().enumerate() {
            if texture.get("source").is_none() {
                let only = texture["extensions"]
                    .as_object()
                    .and_then(|e| e.keys().next().cloned())
                    .unwrap_or_else(|| "no image".into());
                bail!("texture {i} has no PNG or JPEG image (only {only}): {REEXPORT}");
            }
        }
    }
    Ok(())
}

/// What `gltf`'s validation reads before checking it: each primitive's POSITION accessor, which it indexes the
/// accessors by (for her bounds) and would panic on past their end.
fn preflight(root: &gltf::json::Root) -> Result<()> {
    use gltf::json::validation::Checked;
    let position = Checked::Valid(gltf::json::mesh::Semantic::Positions);
    for (m, mesh) in root.meshes.iter().enumerate() {
        for (p, primitive) in mesh.primitives.iter().enumerate() {
            if let Some(index) = primitive.attributes.get(&position) {
                ensure!(
                    index.value() < root.accessors.len(),
                    "invalid glTF: mesh {m} primitive {p}'s POSITION is accessor {}, of {}",
                    index.value(),
                    root.accessors.len()
                );
            }
        }
    }
    Ok(())
}

/// `gltf`'s validation, less its complaint about required extensions (which [`refuse`] has judged).
fn validate(root: &gltf::json::Root) -> Result<()> {
    use gltf::json::validation::{Error, Validate};
    let mut errors = Vec::new();
    root.validate(root, gltf::json::Path::new, &mut |path, error| {
        if error != Error::Unsupported {
            errors.push(format!("{}: {error}", path()));
        }
    });
    if errors.is_empty() {
        return Ok(());
    }
    let more = if errors.len() > 5 {
        format!(" (and {} more)", errors.len() - 5)
    } else {
        String::new()
    };
    errors.truncate(5);
    bail!("invalid glTF: {}{more}", errors.join("; "))
}

/// Runs a read of a file, a panic in it its error: a check here that `gltf`'s readers' assertions outrun must not bring
/// the host down with a downloaded file. (A backstop: the checks above are what keep the readers from panicking.)
pub(crate) fn guarded<T>(read: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|panic| {
        let what = panic
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        bail!("the glTF reader failed on it ({what}): the file is malformed")
    })
}

/// The JSON's array of strings at `value`, or none.
pub(crate) fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// A URI's bytes: a `data:` URI decoded, anything else through the resolver (percent-decoded).
pub(crate) fn fetch(uri: &str, resolver: Resolver<'_>) -> Result<Vec<u8>> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',').context("a data URI with no data")?;
        ensure!(meta.ends_with(";base64"), "a data URI that is not base64");
        return base64(data);
    }
    resolver(&percent_decode(uri)).with_context(|| format!("reading {uri}"))
}

fn percent_decode(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((high * 16 + low) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64(text: &str) -> Result<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut bits, mut count) = (0u32, 0);
    for c in text.bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = value(c).context("a data URI that is not base64")?;
        bits = (bits << 6) | v;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    Ok(out)
}

/// The scene's triangles in its own coordinates (every node's transform applied), by material.
#[derive(Default)]
pub(crate) struct SceneMesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uv0: Vec<[f32; 2]>,
    pub uv1: Vec<[f32; 2]>,
    pub colors: Vec<[u8; 4]>,
    /// Triangle indices by material (`None` glTF's default material), in the order first met.
    pub groups: Vec<(Option<usize>, Vec<u32>)>,
    /// Primitives drawn.
    pub primitives: usize,
    /// What was left out, for the host to show.
    pub notes: Vec<String>,
}

impl SceneMesh {
    pub fn triangles(&self) -> usize {
        self.groups.iter().map(|(_, t)| t.len() / 3).sum()
    }

    fn group(&mut self, material: Option<usize>) -> &mut Vec<u32> {
        let i = match self.groups.iter().position(|(m, _)| *m == material) {
            Some(i) => i,
            None => {
                self.groups.push((material, Vec::new()));
                self.groups.len() - 1
            }
        };
        &mut self.groups[i].1
    }
}

/// Every triangle of the file's scene (its default, or its first; with none, every node that is no node's child).
pub(crate) fn scene_mesh(file: &File) -> Result<SceneMesh> {
    let document = &file.document;
    let roots: Vec<gltf::Node<'_>> = match document
        .default_scene()
        .or_else(|| document.scenes().next())
    {
        Some(scene) => scene.nodes().collect(),
        None => {
            let children: std::collections::HashSet<usize> = document
                .nodes()
                .flat_map(|n| n.children().map(|c| c.index()).collect::<Vec<_>>())
                .collect();
            document
                .nodes()
                .filter(|n| !children.contains(&n.index()))
                .collect()
        }
    };
    let mut mesh = SceneMesh::default();
    let mut skipped_modes = 0usize;
    // A scene is a forest: a node reached twice (two parents, a root listed twice, a cycle) is refused, so the walk
    // visits each node once however its children are listed (a chain listing each child twice would double the
    // visits at every step).
    let mut reached = vec![false; document.nodes().len()];
    let mut stack: Vec<(gltf::Node<'_>, Mat4)> =
        roots.into_iter().map(|n| (n, Mat4::IDENTITY)).collect();
    while let Some((node, parent)) = stack.pop() {
        ensure!(
            !std::mem::replace(&mut reached[node.index()], true),
            "node {} is reached twice in its scene (a glTF node has one parent)",
            node.index()
        );
        let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        ensure!(
            transform.is_finite(),
            "node {} has a transform that is not finite",
            node.index()
        );
        if let Some(m) = node.mesh() {
            for primitive in m.primitives() {
                if !matches!(
                    primitive.mode(),
                    Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan
                ) {
                    skipped_modes += 1;
                    continue;
                }
                add_primitive(file, &primitive, transform, &mut mesh).with_context(|| {
                    format!("mesh {} primitive {}", m.index(), primitive.index())
                })?;
            }
        }
        for child in node.children() {
            stack.push((child, transform));
        }
    }
    if skipped_modes > 0 {
        mesh.notes.push(format!(
            "{skipped_modes} primitives of points or lines left out"
        ));
    }
    ensure!(mesh.triangles() > 0, "its scene has no triangles");
    Ok(mesh)
}

/// What an accessor may hold, read as one thing: its component types (each with whether glTF has it normalised) and
/// its dimensions. `gltf`'s readers trust both.
struct Kind {
    what: &'static str,
    types: &'static [(DataType, bool)],
    dimensions: &'static [Dimensions],
    /// The same, in words.
    allowed: &'static str,
}

const POSITIONS: Kind = Kind {
    what: "positions",
    types: &[(DataType::F32, false)],
    dimensions: &[Dimensions::Vec3],
    allowed: "float vec3",
};
const NORMALS: Kind = Kind {
    what: "normals",
    types: &[(DataType::F32, false)],
    dimensions: &[Dimensions::Vec3],
    allowed: "float vec3",
};
const TEXCOORDS: Kind = Kind {
    what: "UVs",
    types: &[
        (DataType::F32, false),
        (DataType::U8, true),
        (DataType::U16, true),
    ],
    dimensions: &[Dimensions::Vec2],
    allowed: "float or normalised unsigned byte or short vec2",
};
const COLORS: Kind = Kind {
    what: "vertex colours",
    types: &[
        (DataType::F32, false),
        (DataType::U8, true),
        (DataType::U16, true),
    ],
    dimensions: &[Dimensions::Vec3, Dimensions::Vec4],
    allowed: "float or normalised unsigned byte or short vec3 or vec4",
};
const INDICES: Kind = Kind {
    what: "indices",
    types: &[
        (DataType::U8, false),
        (DataType::U16, false),
        (DataType::U32, false),
    ],
    dimensions: &[Dimensions::Scalar],
    allowed: "unsigned byte, short or int scalars",
};

/// Checks that `accessor` holds what `kind` allows and that every byte `gltf`'s readers will touch (its view's, its
/// sparse indices' and values') lies within its buffer, at a stride no shorter than an element; returns its count.
fn check_accessor(file: &File, accessor: &gltf::Accessor<'_>, kind: &Kind) -> Result<usize> {
    let i = accessor.index();
    let (data_type, dimensions) = (accessor.data_type(), accessor.dimensions());
    ensure!(
        kind.types.contains(&(data_type, accessor.normalized()))
            && kind.dimensions.contains(&dimensions),
        "its {} (accessor {i}) are {data_type:?} {dimensions:?}{}, not {} (quantized: {REEXPORT})",
        kind.what,
        if accessor.normalized() {
            " normalised"
        } else {
            ""
        },
        kind.allowed
    );
    let count = accessor.count();
    ensure!(count > 0, "accessor {i} is empty");
    let size = accessor.size();
    if let Some(view) = accessor.view() {
        span(file, &view, accessor.offset(), count, size)
            .with_context(|| format!("accessor {i}"))?;
    }
    if let Some(sparse) = accessor.sparse() {
        let n = sparse.count();
        ensure!(
            (1..=count).contains(&n),
            "accessor {i}'s sparse count is {n}, not 1 to its {count}"
        );
        let indices = sparse.indices();
        span(
            file,
            &indices.view(),
            indices.offset(),
            n,
            indices.index_type().size(),
        )
        .with_context(|| format!("accessor {i}'s sparse indices"))?;
        let values = sparse.values();
        span(file, &values.view(), values.offset(), n, size)
            .with_context(|| format!("accessor {i}'s sparse values"))?;
    }
    Ok(count)
}

/// `count` (at least 1) elements of `size` bytes from `offset` in `view`, at its stride, lie within it and it within its
/// buffer; in checked arithmetic, as a file's numbers may be anything.
fn span(
    file: &File,
    view: &gltf::buffer::View<'_>,
    offset: usize,
    count: usize,
    size: usize,
) -> Result<()> {
    let buffer = file.buffers.get(view.buffer().index()).map_or(0, Vec::len);
    ensure!(
        view.offset()
            .checked_add(view.length())
            .is_some_and(|end| end <= buffer),
        "buffer view {} lies outside its buffer",
        view.index()
    );
    let stride = view.stride().unwrap_or(size);
    ensure!(
        stride >= size,
        "buffer view {}'s stride of {stride} bytes is under an element's {size}",
        view.index()
    );
    let end = count
        .checked_sub(1)
        .and_then(|last| stride.checked_mul(last))
        .and_then(|b| b.checked_add(offset))
        .and_then(|b| b.checked_add(size));
    ensure!(
        end.is_some_and(|end| end <= view.length()),
        "it lies outside its buffer view {}",
        view.index()
    );
    Ok(())
}

fn add_primitive(
    file: &File,
    primitive: &gltf::Primitive<'_>,
    transform: Mat4,
    mesh: &mut SceneMesh,
) -> Result<()> {
    let buffers = &file.buffers;
    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
    let Some(positions) = primitive.get(&gltf::Semantic::Positions) else {
        mesh.notes
            .push("a primitive with no positions left out".to_owned());
        return Ok(());
    };
    let count = check_accessor(file, &positions, &POSITIONS)?;
    // Every count is bounded before it is read: a sparse accessor's few bytes can name billions of elements.
    ensure!(
        count <= MAX_VERTICES.saturating_sub(mesh.positions.len()),
        "over {MAX_VERTICES} vertices: decimate the model"
    );
    let points: Vec<Vec3> = reader
        .read_positions()
        .context("its positions lie outside their buffer")?
        .map(Vec3::from)
        .collect();
    // An attribute's accessor, checked, of as many elements as the positions.
    let attribute = |semantic: gltf::Semantic, kind: &Kind| -> Result<bool> {
        let Some(accessor) = primitive.get(&semantic) else {
            return Ok(false);
        };
        let n = check_accessor(file, &accessor, kind)?;
        ensure!(
            n == count,
            "its {} number {n}, its positions {count}",
            kind.what
        );
        Ok(true)
    };
    let normals: Option<Vec<Vec3>> = if attribute(gltf::Semantic::Normals, &NORMALS)? {
        let normals = reader
            .read_normals()
            .context("its normals lie outside their buffer")?;
        Some(normals.map(Vec3::from).collect())
    } else {
        None
    };
    let uv = |set: u32| -> Result<Vec<[f32; 2]>> {
        Ok(if attribute(gltf::Semantic::TexCoords(set), &TEXCOORDS)? {
            reader
                .read_tex_coords(set)
                .context("its UVs lie outside their buffer")?
                .into_f32()
                .collect()
        } else {
            vec![[0.0; 2]; count]
        })
    };
    let (uv0, uv1) = (uv(0)?, uv(1)?);
    let colors: Vec<[u8; 4]> = if attribute(gltf::Semantic::Colors(0), &COLORS)? {
        reader
            .read_colors(0)
            .context("its vertex colours lie outside their buffer")?
            .into_rgba_u8()
            .collect()
    } else {
        vec![[255; 4]; count]
    };
    ensure!(
        points.len() == count && uv0.len() == count && uv1.len() == count && colors.len() == count,
        "its attributes' lengths differ"
    );
    let corners: Vec<u32> = match primitive.indices() {
        Some(a) => {
            let n = check_accessor(file, &a, &INDICES)?;
            // A strip or a fan of n corners makes n - 2 triangles, a list n / 3.
            let room = MAX_TRIANGLES.saturating_sub(mesh.triangles());
            ensure!(
                n <= room.saturating_mul(3).saturating_add(2),
                "over {MAX_TRIANGLES} triangles: decimate the model"
            );
            reader
                .read_indices()
                .context("its indices lie outside their buffer")?
                .into_u32()
                .collect()
        }
        None => (0..count as u32).collect(),
    };
    ensure!(
        corners.iter().all(|&i| (i as usize) < count),
        "an index past its {count} vertices"
    );
    let mut triangles: Vec<[u32; 3]> = match primitive.mode() {
        Mode::Triangles => corners.as_chunks::<3>().0.to_vec(),
        Mode::TriangleStrip => (2..corners.len())
            .map(|i| {
                if i % 2 == 0 {
                    [corners[i - 2], corners[i - 1], corners[i]]
                } else {
                    [corners[i - 1], corners[i - 2], corners[i]]
                }
            })
            .collect(),
        Mode::TriangleFan => (2..corners.len())
            .map(|i| [corners[0], corners[i - 1], corners[i]])
            .collect(),
        _ => Vec::new(),
    };
    // A mirroring node turns the faces inside out: wind them back.
    if transform.determinant() < 0.0 {
        for t in &mut triangles {
            t.swap(1, 2);
        }
    }
    ensure!(
        mesh.triangles() + triangles.len() <= MAX_TRIANGLES,
        "over {MAX_TRIANGLES} triangles: decimate the model"
    );
    let normal_matrix = Mat3::from_mat4(transform).inverse().transpose();
    let world: Vec<Vec3> = points
        .iter()
        .map(|&p| transform.transform_point3(p))
        .collect();
    ensure!(
        world.iter().all(|p| p.is_finite()),
        "a position that is not finite"
    );
    let material = primitive.material().index();
    let base = mesh.positions.len() as u32;
    match normals {
        Some(normals) => {
            mesh.positions.extend_from_slice(&world);
            mesh.normals.extend(
                normals
                    .iter()
                    .map(|&n| (normal_matrix * n).normalize_or(Vec3::Y)),
            );
            mesh.uv0.extend_from_slice(&uv0);
            mesh.uv1.extend_from_slice(&uv1);
            mesh.colors.extend_from_slice(&colors);
            let group = mesh.group(material);
            group.extend(triangles.iter().flatten().map(|&i| base + i));
        }
        // No normals: glTF's flat shading, each triangle its own three corners (bounded before they are made).
        None => {
            ensure!(
                triangles
                    .len()
                    .checked_mul(3)
                    .is_some_and(|n| n <= MAX_VERTICES.saturating_sub(mesh.positions.len())),
                "over {MAX_VERTICES} vertices: decimate the model"
            );
            let mut indices = Vec::with_capacity(triangles.len() * 3);
            for t in &triangles {
                let [a, b, c] = t.map(|i| world[i as usize]);
                let n = (b - a).cross(c - a).normalize_or(Vec3::Y);
                for &i in t {
                    let i = i as usize;
                    indices.push(mesh.positions.len() as u32);
                    mesh.positions.push(world[i]);
                    mesh.normals.push(n);
                    mesh.uv0.push(uv0[i]);
                    mesh.uv1.push(uv1[i]);
                    mesh.colors.push(colors[i]);
                }
            }
            mesh.group(material).extend(indices);
        }
    }
    mesh.primitives += 1;
    Ok(())
}

/// How a material's surface is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Alpha {
    Opaque,
    /// Cut where its alpha is under the cut-off.
    Mask(f32),
    /// Blended over what is behind it, drawn after the sea.
    Blend,
}

/// A material's maps, in the shaders' order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    BaseColor = 0,
    MetallicRoughness = 1,
    Normal = 2,
    Occlusion = 3,
    Emissive = 4,
}

impl Slot {
    pub const ALL: [Slot; 5] = [
        Slot::BaseColor,
        Slot::MetallicRoughness,
        Slot::Normal,
        Slot::Occlusion,
        Slot::Emissive,
    ];

    /// Colour maps are sRGB; the rest are data.
    pub fn srgb(self) -> bool {
        matches!(self, Slot::BaseColor | Slot::Emissive)
    }

    pub fn name(self) -> &'static str {
        match self {
            Slot::BaseColor => "base colour",
            Slot::MetallicRoughness => "metal-rough",
            Slot::Normal => "normal",
            Slot::Occlusion => "occlusion",
            Slot::Emissive => "emissive",
        }
    }
}

/// A texture's wrapping on one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Wrap {
    Repeat,
    Mirror,
    Clamp,
}

impl Wrap {
    pub fn address_mode(self) -> wgpu::AddressMode {
        match self {
            Wrap::Repeat => wgpu::AddressMode::Repeat,
            Wrap::Mirror => wgpu::AddressMode::MirrorRepeat,
            Wrap::Clamp => wgpu::AddressMode::ClampToEdge,
        }
    }
}

/// A texture's filtering, one way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Filter {
    Nearest,
    Linear,
}

impl Filter {
    fn mode(self) -> wgpu::FilterMode {
        match self {
            Filter::Nearest => wgpu::FilterMode::Nearest,
            Filter::Linear => wgpu::FilterMode::Linear,
        }
    }
}

/// How a map is read: its texture's own sampler, glTF's default (repeating, smooth, between mip levels) where the file
/// leaves it out. Each map keeps its own: a repeating base colour and a clamped emissive map on one material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sampling {
    /// Across (u) and down (v).
    pub wrap: [Wrap; 2],
    /// Magnified, minified.
    pub mag: Filter,
    pub min: Filter,
    /// Between mip levels; `None` reads the largest only.
    pub mip: Option<Filter>,
}

impl Default for Sampling {
    fn default() -> Self {
        Self {
            wrap: [Wrap::Repeat; 2],
            mag: Filter::Linear,
            min: Filter::Linear,
            mip: Some(Filter::Linear),
        }
    }
}

impl Sampling {
    fn of(sampler: &gltf::texture::Sampler<'_>) -> Self {
        use gltf::texture::{MagFilter, MinFilter, WrappingMode};
        let wrap = |mode: WrappingMode| match mode {
            WrappingMode::Repeat => Wrap::Repeat,
            WrappingMode::MirroredRepeat => Wrap::Mirror,
            WrappingMode::ClampToEdge => Wrap::Clamp,
        };
        let (min, mip) = match sampler.min_filter() {
            Some(MinFilter::Nearest) => (Filter::Nearest, None),
            Some(MinFilter::Linear) => (Filter::Linear, None),
            Some(MinFilter::NearestMipmapNearest) => (Filter::Nearest, Some(Filter::Nearest)),
            Some(MinFilter::LinearMipmapNearest) => (Filter::Linear, Some(Filter::Nearest)),
            Some(MinFilter::NearestMipmapLinear) => (Filter::Nearest, Some(Filter::Linear)),
            Some(MinFilter::LinearMipmapLinear) | None => (Filter::Linear, Some(Filter::Linear)),
        };
        Self {
            wrap: [wrap(sampler.wrap_s()), wrap(sampler.wrap_t())],
            mag: match sampler.mag_filter() {
                Some(MagFilter::Nearest) => Filter::Nearest,
                Some(MagFilter::Linear) | None => Filter::Linear,
            },
            min,
            mip,
        }
    }

    /// The GPU's sampler for it: anisotropic only where every filter is smooth (wgpu allows no other).
    pub fn descriptor(&self) -> wgpu::SamplerDescriptor<'static> {
        let smooth = self.mag == Filter::Linear
            && self.min == Filter::Linear
            && self.mip == Some(Filter::Linear);
        wgpu::SamplerDescriptor {
            label: Some("vessel map"),
            address_mode_u: self.wrap[0].address_mode(),
            address_mode_v: self.wrap[1].address_mode(),
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: self.mag.mode(),
            min_filter: self.min.mode(),
            mipmap_filter: match self.mip {
                Some(Filter::Nearest) | None => wgpu::MipmapFilterMode::Nearest,
                Some(Filter::Linear) => wgpu::MipmapFilterMode::Linear,
            },
            // No mipmapping: the largest level alone.
            lod_max_clamp: if self.mip.is_some() { 32.0 } else { 0.0 },
            anisotropy_clamp: if smooth { 8 } else { 1 },
            ..Default::default()
        }
    }
}

/// One map of a material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Map {
    /// The file's image while reading; the model's texture once loaded ([`crate::VesselModel::textures`]).
    pub texture: usize,
    /// Which UV set it reads: the file's; a drawn material's is 0 or 1 ([`check_uv_sets`]).
    pub set: u32,
    /// KHR_texture_transform's matrix, rows: uv' = (a u + b v + c, d u + e v + f).
    pub transform: [[f32; 3]; 2],
    pub sampling: Sampling,
}

/// A material as the shaders draw it.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// Linear RGBA.
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    /// Linear RGB, times KHR_materials_emissive_strength.
    pub emissive: [f32; 3],
    pub normal_scale: f32,
    pub occlusion_strength: f32,
    pub alpha: Alpha,
    pub double_sided: bool,
    /// KHR_materials_unlit: its colour lit evenly by the sun and the sky, without shading.
    pub unlit: bool,
    /// KHR_materials_transmission's share, 0 none: glass, drawn blended with its reflections whole.
    pub transmission: f32,
    pub maps: [Option<Map>; 5],
}

impl Default for Material {
    /// glTF's default material: white, fully metallic and rough, one-sided.
    fn default() -> Self {
        Self {
            name: "default".into(),
            base_color: [1.0; 4],
            metallic: 1.0,
            roughness: 1.0,
            emissive: [0.0; 3],
            normal_scale: 1.0,
            occlusion_strength: 1.0,
            alpha: Alpha::Opaque,
            double_sided: false,
            unlit: false,
            transmission: 0.0,
            maps: [None; 5],
        }
    }
}

const IDENTITY: [[f32; 3]; 2] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

/// A texture reference in the JSON (`{index, texCoord, extensions}`) as a [`Map`] of the file's image.
fn map(file: &File, info: &Value) -> Option<Map> {
    let index = info["index"].as_u64()? as usize;
    let texture = file.document.textures().nth(index)?;
    let mut set = info["texCoord"].as_u64().unwrap_or(0) as u32;
    let mut transform = IDENTITY;
    let ext = &info["extensions"]["KHR_texture_transform"];
    if ext.is_object() {
        let pair =
            |v: &Value, default: f32| [0, 1].map(|k| v[k].as_f64().map_or(default, |x| x as f32));
        let offset = pair(&ext["offset"], 0.0);
        let scale = pair(&ext["scale"], 1.0);
        let rotation = ext["rotation"].as_f64().unwrap_or(0.0) as f32;
        let (sin, cos) = rotation.sin_cos();
        transform = [
            [cos * scale[0], sin * scale[1], offset[0]],
            [-sin * scale[0], cos * scale[1], offset[1]],
        ];
        if let Some(over) = ext["texCoord"].as_u64() {
            set = over as u32;
        }
    }
    Some(Map {
        texture: texture.source().index(),
        set,
        transform,
        sampling: Sampling::of(&texture.sampler()),
    })
}

/// Refuses a drawn material with a map on a UV set past TEXCOORD_1: a vertex carries two.
pub(crate) fn check_uv_sets(materials: &[Material], mesh: &SceneMesh) -> Result<()> {
    for (material, _) in &mesh.groups {
        let Some(m) = material.and_then(|i| materials.get(i)) else {
            continue;
        };
        for slot in Slot::ALL {
            if let Some(map) = m.maps[slot as usize]
                && map.set > 1
            {
                bail!(
                    "material {:?}'s {} map reads TEXCOORD_{}: only TEXCOORD_0 and TEXCOORD_1 are read (move the map \
                     to one of those UV maps and re-export it)",
                    m.name,
                    slot.name(),
                    map.set
                );
            }
        }
    }
    Ok(())
}

fn number(value: &Value, default: f32) -> f32 {
    value.as_f64().map_or(default, |v| v as f32)
}

fn color<const N: usize>(value: &Value, default: [f32; N]) -> [f32; N] {
    let mut out = default;
    if let Some(a) = value.as_array() {
        for (k, slot) in out.iter_mut().enumerate() {
            if let Some(v) = a.get(k).and_then(Value::as_f64) {
                *slot = v as f32;
            }
        }
    }
    out
}

/// The file's materials, in its order.
pub(crate) fn materials(file: &File) -> Vec<Material> {
    let none = Value::Null;
    let list = file.json["materials"].as_array();
    (0..list.map_or(0, Vec::len))
        .map(|i| {
            let m = list.map_or(&none, |l| &l[i]);
            let pbr = &m["pbrMetallicRoughness"];
            let ext = &m["extensions"];
            let mut material = Material {
                name: m["name"]
                    .as_str()
                    .map_or_else(|| format!("material {i}"), str::to_owned),
                base_color: color(&pbr["baseColorFactor"], [1.0; 4]),
                metallic: number(&pbr["metallicFactor"], 1.0),
                roughness: number(&pbr["roughnessFactor"], 1.0),
                emissive: color(&m["emissiveFactor"], [0.0; 3]),
                normal_scale: number(&m["normalTexture"]["scale"], 1.0),
                occlusion_strength: number(&m["occlusionTexture"]["strength"], 1.0),
                alpha: match m["alphaMode"].as_str() {
                    Some("MASK") => Alpha::Mask(number(&m["alphaCutoff"], 0.5)),
                    Some("BLEND") => Alpha::Blend,
                    _ => Alpha::Opaque,
                },
                double_sided: m["doubleSided"].as_bool().unwrap_or(false),
                unlit: ext.get("KHR_materials_unlit").is_some(),
                transmission: 0.0,
                maps: [None; 5],
            };
            material.maps[Slot::BaseColor as usize] = map(file, &pbr["baseColorTexture"]);
            material.maps[Slot::MetallicRoughness as usize] =
                map(file, &pbr["metallicRoughnessTexture"]);
            material.maps[Slot::Normal as usize] = map(file, &m["normalTexture"]);
            material.maps[Slot::Occlusion as usize] = map(file, &m["occlusionTexture"]);
            material.maps[Slot::Emissive as usize] = map(file, &m["emissiveTexture"]);
            // The older specular-glossiness workflow (many Sketchfab exports), where no metal-rough fallback is given:
            // its diffuse as the base colour, a dielectric of its glossiness.
            let sg = &ext["KHR_materials_pbrSpecularGlossiness"];
            if sg.is_object() && !pbr.is_object() {
                material.base_color = color(&sg["diffuseFactor"], [1.0; 4]);
                material.maps[Slot::BaseColor as usize] = map(file, &sg["diffuseTexture"]);
                material.metallic = 0.0;
                material.roughness = 1.0 - number(&sg["glossinessFactor"], 1.0);
            }
            let strength = number(
                &ext["KHR_materials_emissive_strength"]["emissiveStrength"],
                1.0,
            );
            material.emissive = material.emissive.map(|c| c * strength);
            // Glass: what it lets through is drawn as blending over what is behind it, its reflections kept.
            let transmission = number(
                &ext["KHR_materials_transmission"]["transmissionFactor"],
                0.0,
            );
            if transmission > 0.0 && material.alpha == Alpha::Opaque {
                material.alpha = Alpha::Blend;
                material.transmission = transmission.min(1.0);
                material.base_color[3] *= 1.0 - 0.85 * transmission.min(1.0);
            }
            material
        })
        .collect()
}

/// An image's encoded bytes and a name for it.
pub(crate) fn image_bytes(
    file: &File,
    image: usize,
    resolver: Resolver<'_>,
) -> Result<(Vec<u8>, String)> {
    let image = file
        .document
        .images()
        .nth(image)
        .with_context(|| format!("image {image} is not in the file"))?;
    let name = image
        .name()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("image {}", image.index()));
    let bytes = match image.source() {
        gltf::image::Source::View { view, .. } => {
            let buffer = &file.buffers[view.buffer().index()];
            view.offset()
                .checked_add(view.length())
                .and_then(|end| buffer.get(view.offset()..end))
                .with_context(|| format!("{name} lies outside its buffer"))?
                .to_vec()
        }
        gltf::image::Source::Uri { uri, .. } => {
            let name = if uri.starts_with("data:") {
                name.clone()
            } else {
                uri.to_owned()
            };
            return Ok((fetch(uri, resolver)?, name));
        }
    };
    Ok((bytes, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_uris_and_escapes_decode() {
        assert_eq!(base64("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        assert_eq!(base64("AAEC").unwrap(), vec![0, 1, 2]);
        assert!(base64("a$b").is_err());
        assert_eq!(percent_decode("hull%20base.png"), "hull base.png");
        assert_eq!(percent_decode("100%"), "100%");
        let mut none = |_: &str| -> Result<Vec<u8>> { bail!("no files") };
        assert_eq!(
            fetch("data:application/octet-stream;base64,AAEC", &mut none).unwrap(),
            vec![0, 1, 2]
        );
        assert!(fetch("hull.bin", &mut none).is_err());
    }
}
