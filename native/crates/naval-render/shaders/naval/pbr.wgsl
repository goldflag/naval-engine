// Physically based shading's microfacet terms, as three.js r185 writes them (`three/src/nodes/functions/BSDF/`:
// `F_Schlick`, `V_GGX_SmithCorrelated`, `D_GGX`): the ships' paint (`naval_ships::bsdf`) and the land
// (`naval_terrain::lighting`) are lit with them.
#define_import_path naval::pbr

const RECIPROCAL_PI: f32 = 0.3183098861837907;
// three's `EPSILON` (MathNode.js).
const EPSILON: f32 = 1e-6;

fn f_schlick(f0: vec3<f32>, f90: f32, dot_vh: f32) -> vec3<f32> {
    // Epic's spherical-Gaussian variant of Schlick's approximation.
    let fresnel = exp2((dot_vh * -5.55473 - 6.98316) * dot_vh);
    return f0 * (1.0 - fresnel) + f90 * fresnel;
}

fn v_ggx_smith_correlated(alpha: f32, dot_nl: f32, dot_nv: f32) -> f32 {
    let a2 = alpha * alpha;
    let gv = dot_nl * sqrt(a2 + (1.0 - a2) * dot_nv * dot_nv);
    let gl = dot_nv * sqrt(a2 + (1.0 - a2) * dot_nl * dot_nl);
    return 0.5 / max(gv + gl, EPSILON);
}

fn d_ggx(alpha: f32, dot_nh: f32) -> f32 {
    let a2 = alpha * alpha;
    let denom = 1.0 - dot_nh * dot_nh * (1.0 - a2);
    return a2 / (denom * denom) * RECIPROCAL_PI;
}
