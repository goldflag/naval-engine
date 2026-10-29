// A tree's surface close to, which the near trees' meshes (`near_trees.wgsl`) and the impostors' bake
// (`tree_bake.wgsl`) share, so a tree baked into its impostor looks as its mesh does: a spray's leaves (or bare twigs)
// from the leaf sprites (`naval_terrain::tree_meshes::leaf_sprites`), its twigs in the bark's colour; a limb's bark by
// its species' pattern (`tree_meshes::Bark`) over its coordinates, its relief tilting the normal.
#define_import_path naval_terrain::tree_surface

const PI: f32 = 3.141592653589793;
// The red a sprite's twigs are painted under (`tree_meshes::TWIG_RED`).
const TWIG_RED: f32 = 0.09;
// The leaves' level: a leaf's albedo is its card's colour times its brightness on the sprite (0.4-1.6) times this,
// set so a wood's undersides and canopy seen from under it draw within a tenth of the old trees' green (at 1 a wood
// from inside drew 0.87-0.89 of it), its crowns from afar within a tenth over.
const LEAF_LEVEL: f32 = 1.06;
// A leafy spray's twigs: grey-brown bark.
const TWIG_ALBEDO: vec3<f32> = vec3<f32>(0.085, 0.068, 0.052);
// A limb's coordinates (`tree_meshes::tube`): ridges round it over this, metres along it over this.
const BARK_RIDGES: f32 = 128.0;
const BARK_ALONG_M: f32 = 64.0;

// sRGB to linear, by a cubic within a fraction of a level of the exact curve (a pow a channel is dear on every pixel);
// `trees.wgsl` decodes the impostors' colour with it too.
fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    return c * (c * (c * 0.305306011 + 0.682171111) + 0.012522878);
}

fn surface_hash(p: vec2<f32>) -> f32 {
    var v = bitcast<vec2<u32>>(vec2<i32>(floor(p))) * 1664525u + 1013904223u;
    v.x += v.y * 1664525u;
    v.y += v.x * 1013904223u;
    v = v ^ (v >> vec2<u32>(16u));
    return f32((v.x + v.y * 747796405u) >> 8u) / 16777216.0;
}

// Value noise over a plane, 0-1.
fn surface_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let a = surface_hash(i);
    let b = surface_hash(i + vec2<f32>(1.0, 0.0));
    let c = surface_hash(i + vec2<f32>(0.0, 1.0));
    let d = surface_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

struct TreeSurface {
    // Linear albedo, before the tree's tint and the land's tone.
    albedo: vec3<f32>,
    normal: vec3<f32>,
    // 1 a leaf, 0 bark or a twig.
    leafy: f32,
}

// A spray's pixel: `leaf` its sprite's texel, `color` the card's sRGB albedo, `normal` its clump's normal, `across` the
// card's direction across its sprite, `bare` whether the card carries bare twigs, `tilt` how far each leaf's own tilt
// on its card turns the clump's normal (detail: the clump's light and shade stay the clump's, most of all where a far
// crown averages many leaves into a texel), `back` whether the card is seen from behind (its leaves' tilt mirrored;
// the clump's normal is the clump's from either side).
fn spray_surface(
    leaf: vec4<f32>,
    color: vec3<f32>,
    normal: vec3<f32>,
    across: vec3<f32>,
    bare: bool,
    tilt: f32,
    back: bool,
) -> TreeSurface {
    var s: TreeSurface;
    if bare || leaf.r < TWIG_RED {
        let tone = clamp(leaf.r / TWIG_RED, 0.3, 1.0) * 1.15;
        s.albedo = select(TWIG_ALBEDO, srgb_decode(color), bare) * tone;
        s.leafy = 0.0;
    } else {
        s.albedo = srgb_decode(color) * (0.4 + 1.2 * leaf.r) * LEAF_LEVEL;
        s.leafy = 1.0;
    }
    // Each leaf (and each twig, round) tilted on its card as the sprite says.
    let t = normalize(across - normal * dot(across, normal) + vec3<f32>(1e-6, 0.0, 0.0));
    let b = cross(normal, t);
    s.normal = normalize(normal + (t * (leaf.g - 0.5) + b * (leaf.b - 0.5)) * (2.0 * select(tilt, -tilt, back)));
    return s;
}

// The bark's relief (0 in a furrow, 1 on a ridge's top) and its brightness at `p` (ridges round, metres along), by
// its pattern.
fn bark_relief(style: u32, p: vec2<f32>) -> vec2<f32> {
    switch style {
        // Smooth, a little mottled.
        case 1u: {
            let n = surface_noise(p * vec2<f32>(0.6, 0.5));
            let fine = surface_noise(p * vec2<f32>(2.5, 3.0));
            return vec2<f32>(0.5 + 0.2 * (fine - 0.5), 0.82 + 0.3 * n + 0.08 * fine);
        }
        // Birch: white, dark streaks across it and dark patches; dark and fissured about its foot.
        case 2u: {
            let streak = surface_noise(p * vec2<f32>(0.9, 11.0));
            let blot = surface_noise(p * vec2<f32>(0.35, 0.9));
            let marks = max(smoothstep(0.78, 0.86, streak), smoothstep(0.72, 0.8, blot) * 0.85);
            let foot = smoothstep(1.6, 0.3, p.y);
            let fissure = abs(sin(PI * (p.x * 3.0 + 0.8 * surface_noise(p * vec2<f32>(1.5, 2.0)))));
            let relief = mix(0.5, pow(fissure, 0.4), foot);
            return vec2<f32>(relief, mix(1.0 - 0.8 * marks, 0.25 + 0.35 * relief, foot));
        }
        // Scaly plates parted by dark fissures (the pines).
        case 3u: {
            let row = p.y * 2.4 + 0.6 * surface_noise(p * vec2<f32>(0.7, 1.1));
            let r = floor(row);
            let x = p.x * 0.8 + surface_hash(vec2<f32>(r, 7.0)) * 3.0 + 0.5 * surface_noise(p * vec2<f32>(0.9, 1.7));
            let cx = fract(x);
            let cy = fract(row);
            let edge = min(min(cx, 1.0 - cx) * 2.2, min(cy, 1.0 - cy) * 3.0);
            let relief = smoothstep(0.0, 0.3, edge);
            let tone = 0.78 + 0.45 * surface_hash(vec2<f32>(floor(x), r));
            return vec2<f32>(relief, mix(0.3, tone, relief));
        }
        // Long fibrous strips (redwood, the cypresses).
        case 4u: {
            let wander = surface_noise(p * vec2<f32>(0.5, 0.2));
            let x = p.x + 0.8 * (wander - 0.5);
            let strip = abs(sin(PI * x));
            let relief = pow(strip, 0.5);
            let tone = 0.75 + 0.4 * surface_noise(vec2<f32>(x * 1.0, p.y * 0.4));
            return vec2<f32>(relief, mix(0.45, tone, relief));
        }
        // Rings: a palm's old leaf bases, a bamboo's nodes.
        case 5u: {
            let y = p.y / 0.17 + 0.25 * sin(p.x * 0.7 + surface_noise(p * vec2<f32>(0.3, 0.6)) * 4.0);
            let f = fract(y);
            let ring = smoothstep(0.0, 0.1, f) * smoothstep(0.0, 0.3, 1.0 - f);
            let tone = 0.85 + 0.25 * surface_noise(p * vec2<f32>(0.4, 0.5));
            return vec2<f32>(ring, mix(0.5, tone, ring));
        }
        // Furrowed: ridges up the trunk parted by furrows that wander, split and join, the ridges broken across into
        // blocks.
        default: {
            let wander = surface_noise(p * vec2<f32>(0.5, 0.8)) + 0.5 * surface_noise(p * vec2<f32>(1.1, 2.3));
            let x = p.x + 1.1 * (wander - 0.75);
            let ridge = abs(sin(PI * x));
            let breaks = surface_noise(p * vec2<f32>(1.3, 4.5));
            let relief = (1.0 - pow(1.0 - ridge, 2.5)) * (0.75 + 0.25 * smoothstep(0.25, 0.6, breaks));
            let tone = (0.8 + 0.35 * breaks) * (0.88 + 0.24 * surface_noise(p * vec2<f32>(0.2, 0.25)));
            return vec2<f32>(relief, mix(0.45, tone, relief));
        }
    }
}

// A limb's pixel: `style` its pattern, `uv` its coordinates, `color` its sRGB albedo, `normal` its normal (facing the
// eye), `along` its direction along the limb.
fn bark_surface(style: u32, uv: vec2<f32>, color: vec3<f32>, normal: vec3<f32>, along: vec3<f32>) -> TreeSurface {
    let p = vec2<f32>(uv.x * BARK_RIDGES, uv.y * BARK_ALONG_M);
    let here = bark_relief(style, p);
    // The relief's slope round the limb and along it, tilting the normal.
    let round_slope = bark_relief(style, p + vec2<f32>(0.08, 0.0)).x - here.x;
    let along_slope = bark_relief(style, p + vec2<f32>(0.0, 0.04)).x - here.x;
    let t = normalize(along - normal * dot(along, normal) + vec3<f32>(1e-6, 0.0, 0.0));
    let about = cross(normal, t);
    var s: TreeSurface;
    s.normal = normalize(normal - about * (round_slope * 4.0) - t * (along_slope * 3.0));
    s.albedo = srgb_decode(color) * here.y;
    s.leafy = 0.0;
    return s;
}
