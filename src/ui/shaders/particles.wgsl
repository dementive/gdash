// Image-sourced particles (see particles.rs). Premultiplied output: glowing particles add
// light (alpha 0), leaves cover what's behind them.
//
// uv.x carries the shape in its tens (0 soft dot / streak, 1 star glint, 2 leaf) and the quad
// corner (-1..1) in the rest; uv.y is the other corner coordinate.

struct FragIn {
    @location(2) uv: vec2<f32>,
    @location(4) color: vec4<f32>,
};

@fragment
fn fragment(in: FragIn) -> @location(0) vec4<f32> {
    let shape = round(in.uv.x / 10.0);
    let p = vec2<f32>(in.uv.x - shape * 10.0, in.uv.y);
    let a = in.color.a;
    if (shape < 0.5) {
        // Soft dot with a hot core.
        let r2 = dot(p, p);
        let m = exp(-r2 * 4.0) * 0.6 + exp(-r2 * 18.0) * 0.8;
        return vec4<f32>(in.color.rgb * m * a, 0.0);
    } else if (shape < 1.5) {
        // Glint: a bright core with four thin spikes.
        let r2 = dot(p, p);
        let core = exp(-r2 * 60.0) * 1.2 + exp(-r2 * 12.0) * 0.25;
        let sx = exp(-abs(p.y) * 70.0) * pow(max(1.0 - abs(p.x), 0.0), 2.0);
        let sy = exp(-abs(p.x) * 70.0) * pow(max(1.0 - abs(p.y), 0.0), 2.0);
        let m = core + 0.7 * (sx + sy);
        return vec4<f32>(in.color.rgb * m * a, 0.0);
    }
    // Leaf: a soft-edged ellipse that covers (normal blending).
    let e = dot(p, p);
    let m = smoothstep(1.0, 0.7, e) * a;
    return vec4<f32>(in.color.rgb * m, m);
}
