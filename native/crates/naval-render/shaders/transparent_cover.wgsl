// The transparents' cover: drawn first in the scene's `Transparent` phase while the display pass grades the night
// (`display_pass.wgsl`, `scene_radiance`), it zeroes the scene's alpha, which nothing drawn before reads, so the alpha
// the transparents' blending leaves (each `a + A (1 - a)`) is how much of a pixel smoke, spray and fire cover. Blended
// by `min`, it also holds the opaque scene's light under `HELD`: after dark the lights come lifted with the night
// (`naval_render::lights::night_lift`), and a deck beside a heavy turret's flash, or its paint's highlight, takes more
// than the scene's half floats hold (65,504), which they would keep as infinity, and a smoke drawn over it at full cover
// as not a number. The same triangle, its alpha left alone (`DisplayPass::draw_hold`), holds the scene so before each
// copy other layers read (the opaque scene the sea looks through, the backdrop the blast waves bend). No TypeScript
// source: the web game graded its night after the scene.
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    return fullscreen_vertex(index, 0.0);
}

// The most light the opaque scene keeps (the scene's units): under a half float's 65,504, with room for the
// transparents over it.
const HELD: f32 = 32768.0;

@fragment
fn fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(HELD, HELD, HELD, 0.0);
}
