// Overlay glass. Bevy has no backdrop blur, so the background is rendered a second time
// small (see ui/glass.rs) and this material samples that copy where the card sits on
// screen, blurs it further, lifts the saturation and lays a tinted pane over it — the
// `backdrop-filter: blur(26px) saturate(1.2)` of the mock-ups, but alive: the picture
// behind the panel keeps breathing while the panel is open.

import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_render::view::{View, frag_coord_to_uv};

@group(0) @binding(0)
var<uniform> view: View;

struct GlassMaterial {
    /// x: reveal, 0 = the flat dark card, 1 = full glass.
    params: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> material: GlassMaterial;
@group(1) @binding(1)
var backdrop: texture_2d<f32>;
@group(1) @binding(2)
var backdrop_s: sampler;

fn sd_rounded_box(p: vec2<f32>, size: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - 0.5 * size + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    // The pane's rounded shape, antialiased.
    let p = (in.uv - vec2<f32>(0.5)) * in.size;
    let r = clamp(max(in.border_radius_x.x, in.border_radius_y.x), 0.0, 0.5 * min(in.size.x, in.size.y));
    let cover = saturate(0.5 - sd_rounded_box(p, in.size, r));
    if (cover <= 0.0) {
        return vec4<f32>(0.0);
    }

    // Where this pixel sits on screen, in the backdrop copy.
    let uv = frag_coord_to_uv(in.position.xy, view.viewport);
    let dims = vec2<f32>(textureDimensions(backdrop));

    // A golden-angle disc of taps over the copy — already small and softened by the
    // downscale — reads as the mock-ups' 26-px blur. UVs clamp at the pane's edges the
    // way backdrop-filter does, so the light bends around the frame.
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 13; i = i + 1) {
        let a = f32(i) * 2.39996;
        let rr = sqrt((f32(i) + 0.5) / 13.0) * 6.5;
        let off = vec2<f32>(cos(a), sin(a)) * rr / dims;
        sum += textureSampleLevel(backdrop, backdrop_s, clamp(uv + off, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
    }
    var col = (sum + textureSampleLevel(backdrop, backdrop_s, uv, 0.0).rgb) / 14.0;

    // Saturate and lift a touch: glass keeps a little more life in the light than the
    // picture behind it, so the pane never reads as a hole in the scene.
    let luma = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
    col = mix(vec3<f32>(luma), col, 1.22) * 1.12;

    // The pane itself: cool ink, heavier toward the bottom where the body text sits,
    // with a soft sheen running along the top edge.
    let y = in.uv.y;
    col = mix(col, vec3<f32>(0.004, 0.007, 0.013), mix(0.56, 0.74, y));
    col += vec3<f32>(0.030 * (1.0 - smoothstep(0.0, 0.16, y)));

    // Racks in from the flat dark card as the overlay opens.
    let dark = mix(vec3<f32>(0.023, 0.030, 0.044), vec3<f32>(0.003, 0.005, 0.009), y);
    col = mix(dark, col, material.params.x);

    return vec4<f32>(col, cover);
}
