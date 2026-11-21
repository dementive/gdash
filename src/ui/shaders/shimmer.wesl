// A slow light sweep across a card whose artwork is still on its way. Not a spinner:
// the card breathes and a soft band crosses it every few seconds, seeded per game so a
// whole shelf never sweeps in lockstep.

import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_render::globals::Globals;

@group(0) @binding(1)
var<uniform> globals: Globals;

struct ShimmerMaterial {
    /// x: seed, staggers the sweep per card.
    params: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> material: ShimmerMaterial;

fn sd_rounded_box(p: vec2<f32>, size: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - 0.5 * size + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let p = (in.uv - vec2<f32>(0.5)) * in.size;
    let r = clamp(max(in.border_radius_x.x, in.border_radius_y.x), 0.0, 0.5 * min(in.size.x, in.size.y));
    let cover = saturate(0.5 - sd_rounded_box(p, in.size, r));
    if (cover <= 0.0) {
        return vec4<f32>(0.0);
    }

    let t = globals.time + material.params.x * 37.0;
    // A diagonal band crossing every ~3.5 seconds, soft as a reflection moving over ink.
    let x = in.uv.x - in.uv.y * 0.55;
    let d = abs(fract(x - t * 0.30) - 0.5);
    let band = exp(-d * d / 0.010);
    // A faint breathing glow between sweeps, so a waiting card still feels tended.
    let breathe = 0.5 + 0.5 * sin(t * 1.4);
    let a = cover * (0.05 + 0.045 * breathe + 0.13 * band);
    return vec4<f32>(vec3<f32>(0.78, 0.85, 1.0), a);
}
