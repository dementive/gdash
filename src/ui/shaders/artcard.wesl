// The details sheet's living art header: the game's hero image, cover-fit into the card
// and drifting slowly around the scene's focal point — the same picture the showcase
// holds defocused behind the sheet, here crisp and close. Rounds itself to the node's
// border radius like the other UI materials.

import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_render::globals::Globals;

@group(0) @binding(1)
var<uniform> globals: Globals;

struct ArtCardMaterial {
    /// x: time, y: reveal (0..1), z/w: the focal point (uv) the drift circles.
    params: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> material: ArtCardMaterial;
@group(1) @binding(1)
var hero: texture_2d<f32>;
@group(1) @binding(2)
var hero_s: sampler;

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

    let t = material.params.x;
    let reveal = material.params.y;
    let focal = material.params.zw;
    let size = vec2<f32>(textureDimensions(hero));
    let box_asp = in.size.x / in.size.y;
    let tex_asp = size.x / max(size.y, 1.0);
    // Cover-fit window (uv): the whole shorter axis, the rest cropped.
    var window = select(
        vec2<f32>(box_asp / tex_asp, 1.0),
        vec2<f32>(1.0, tex_asp / box_asp),
        tex_asp < box_asp,
    );
    // A touch of zoom that settles as the sheet racks in, like the launch stamp.
    window /= 1.06 + 0.05 * (1.0 - reveal);
    // A slow lap around the focal point, tight enough to read as one held shot.
    let drift = vec2<f32>(sin(t * 0.16), 0.7 * cos(t * 0.125)) * 0.016;
    let center = clamp(focal + drift, window * 0.5, 1.0 - window * 0.5);
    let uv = center - window * 0.5 + in.uv * window;
    var col = textureSampleLevel(hero, hero_s, uv, 0.0).rgb;
    // Ground it: the bottom of the card sinks slightly, where the title follows.
    col *= 1.0 - 0.22 * smoothstep(0.72, 1.0, in.uv.y);
    return vec4<f32>(col, cover * reveal);
}
