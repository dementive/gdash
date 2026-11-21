// Showcase background (Material2d on an HDR camera).
//
// - Hero art shown across the full width, anchored to the top and fading out below. Its
//   depth map is snapped into a few flat layers (cut-out cards), so motion separates the
//   layers while every subject stays rigid. Each game has a motion style (breathe, glide or
//   push-in, chosen on the CPU) around the focal point the depth model found.
// - The picture itself comes alive where the scene analysis found something: fire flickers
//   and glows, lights and stars twinkle, water catches a moving sheen, clouds drift a little. (Embers, glints and
//   leaves coming off the image are particles drawn above this; see particles.wgsl.)
// - Per-game atmosphere: light beams from the top corners, and one of several particle
//   styles (dust, snow/ash, bokeh, sparks, or none), tinted with the game's colour.
// - Darkened only where the UI needs contrast. Values above 1.0 are left for bloom.

struct FragIn {
    @location(2) uv: vec2<f32>,
};

@group(2) @binding(0) var<uniform> tint: vec4<f32>;      // rgb: game accent (linear)
@group(2) @binding(1) var<uniform> params: vec4<f32>;    // x: time, y: mix a→b, z: has a, w: has b
@group(2) @binding(2) var<uniform> sizes: vec4<f32>;     // hero a (w,h), hero b (w,h)
@group(2) @binding(3) var<uniform> zoom: vec4<f32>;      // x: zoom a, y: zoom b, zw: screen size (physical px)
@group(2) @binding(4) var hero_a: texture_2d<f32>;
@group(2) @binding(5) var hero_a_s: sampler;
@group(2) @binding(6) var hero_b: texture_2d<f32>;
@group(2) @binding(7) var hero_b_s: sampler;
@group(2) @binding(8) var<uniform> fx: vec4<f32>;        // x: launch flash, y: kick, zw: flash origin
@group(2) @binding(9) var<uniform> look: vec4<f32>;      // xy: parallax camera, zw: has depth a/b
@group(2) @binding(10) var<uniform> style: vec4<f32>;    // x: style, y: amount, z: seed, w: beam tint
@group(2) @binding(11) var<uniform> beams: vec4<f32>;    // x: left, y: right
@group(2) @binding(12) var<uniform> breath: vec4<f32>;   // x/y: depth dolly a / b, z/w: relight a / b
@group(2) @binding(13) var<uniform> focal: vec4<f32>;    // focal point a (xy), b (zw)
@group(2) @binding(14) var depth_a: texture_2d<f32>;
@group(2) @binding(15) var depth_a_s: sampler;
@group(2) @binding(16) var depth_b: texture_2d<f32>;
@group(2) @binding(17) var depth_b_s: sampler;
@group(2) @binding(18) var<uniform> motion: vec4<f32>;   // parallax camera a (xy), b (zw)
// Effect maps from the scene analysis: R fire, G twinkle (lights, stars), B water, A sky.
@group(2) @binding(19) var fxmap_a: texture_2d<f32>;
@group(2) @binding(20) var fxmap_a_s: sampler;
@group(2) @binding(21) var fxmap_b: texture_2d<f32>;
@group(2) @binding(22) var fxmap_b_s: sampler;
// x: 0 = showcase (image across the top, UI shading), 1 = backdrop (image fills the screen,
// for the All grid). In between while switching.
@group(2) @binding(23) var<uniform> backdrop: vec4<f32>;

const INK: vec3<f32> = vec3<f32>(0.0009, 0.0015, 0.0024); // #030508 in linear
// Camera movement in image UV: part moves everything, part separates the depth layers.
const CAM_RIGID: f32 = 0.004;
const CAM_PARALLAX: f32 = 0.004;
// How much light swells on near subjects (and recedes from the background).
const RELIGHT: f32 = 0.14;
// The image covers at least this much of the screen height; wide art keeps its full width.
const MIN_COVER: f32 = 0.62;
// Overscan, so the parallax never samples past the image's sides.
const OVERSCAN: f32 = 1.045;

// Screen pixels per image pixel for the layout below.
fn hero_scale(screen: vec2<f32>, tex: vec2<f32>) -> f32 {
    return max(screen.x / tex.x, screen.y * MIN_COVER / tex.y) * OVERSCAN;
}

// How the screen maps onto the image, as (cx, y0, shown.x, shown.y): a screen point uv lands
// at (cx + (uv.x - 0.5) * shown.x, y0 + uv.y * shown.y).
// Showcase: full width, top-anchored, taller images cropped around the focal point (y > 1 is
// below the image). Backdrop: covers the whole screen, centred on the focal point.
fn layout_params(screen: vec2<f32>, tex: vec2<f32>, f: vec2<f32>) -> vec4<f32> {
    let shown1 = screen / (tex * hero_scale(screen, tex));
    let cx1 = clamp(f.x, shown1.x * 0.5, 1.0 - shown1.x * 0.5);
    var y01 = 0.0;
    if (shown1.y < 1.0) {
        y01 = clamp(f.y - shown1.y * 0.5, 0.0, 1.0 - shown1.y);
    }
    let shown2 = screen / (tex * max(screen.x / tex.x, screen.y / tex.y) * OVERSCAN);
    let cx2 = clamp(f.x, shown2.x * 0.5, 1.0 - shown2.x * 0.5);
    let cy2 = clamp(f.y, shown2.y * 0.5, 1.0 - shown2.y * 0.5);
    return mix(vec4<f32>(cx1, y01, shown1), vec4<f32>(cx2, cy2 - 0.5 * shown2.y, shown2), backdrop.x);
}

fn hero_uv(uv: vec2<f32>, screen: vec2<f32>, tex: vec2<f32>, z: f32, f: vec2<f32>) -> vec2<f32> {
    let p = layout_params(screen, tex, f);
    let base = vec2<f32>(p.x + (uv.x - 0.5) * p.z, p.y + uv.y * p.w);
    return f + (base - f) / z;
}

// Catmull-Rom bicubic from 9 bilinear taps: much crisper than plain bilinear when the
// image is magnified, without bilinear's diamond artifacts.
fn sample_bicubic(t: texture_2d<f32>, s: sampler, uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(t));
    let p = uv * size - 0.5;
    let tc = floor(p) + 0.5;
    let f = p - floor(p);
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let t0 = (tc - 1.0) / size;
    let t3 = (tc + 2.0) / size;
    let t12 = (tc + w2 / w12) / size;
    var c = textureSampleLevel(t, s, vec2<f32>(t0.x, t0.y), 0.0).rgb * w0.x * w0.y;
    c += textureSampleLevel(t, s, vec2<f32>(t12.x, t0.y), 0.0).rgb * w12.x * w0.y;
    c += textureSampleLevel(t, s, vec2<f32>(t3.x, t0.y), 0.0).rgb * w3.x * w0.y;
    c += textureSampleLevel(t, s, vec2<f32>(t0.x, t12.y), 0.0).rgb * w0.x * w12.y;
    c += textureSampleLevel(t, s, vec2<f32>(t12.x, t12.y), 0.0).rgb * w12.x * w12.y;
    c += textureSampleLevel(t, s, vec2<f32>(t3.x, t12.y), 0.0).rgb * w3.x * w12.y;
    c += textureSampleLevel(t, s, vec2<f32>(t0.x, t3.y), 0.0).rgb * w0.x * w3.y;
    c += textureSampleLevel(t, s, vec2<f32>(t12.x, t3.y), 0.0).rgb * w12.x * w3.y;
    c += textureSampleLevel(t, s, vec2<f32>(t3.x, t3.y), 0.0).rgb * w3.x * w3.y;
    return max(c, vec3<f32>(0.0));
}

// Low-resolution art stretched a lot: a blur reads as "dreamy" instead of showing JPEG
// blocks and stair-stepped edges. Tiny images (old console screenshots) get a wide blur and
// become a soft colour wash of the scene.
fn sample_soft(t: texture_2d<f32>, s: sampler, uv: vec2<f32>, up: f32) -> vec3<f32> {
    let r = mix(1.2, 3.0, smoothstep(5.0, 12.0, up)) / vec2<f32>(textureDimensions(t));
    var c = textureSampleLevel(t, s, uv, 0.0).rgb * 0.2;
    for (var i = 0; i < 8; i++) {
        let a = f32(i) * 0.7853982;
        c += textureSampleLevel(t, s, uv + vec2<f32>(cos(a), sin(a)) * r, 0.0).rgb * 0.1;
    }
    return c;
}

// Hero colour (rgb) and nearness (a), displaced by depth against the parallax camera.
// `up` is how many screen pixels each image pixel covers.
fn hero_sample(t: texture_2d<f32>, s: sampler, dt: texture_2d<f32>, ds: sampler, ft: texture_2d<f32>, fs: sampler, uv0: vec2<f32>, has_depth: f32, up: f32, f: vec2<f32>, dolly: f32, cam: vec2<f32>, br: f32) -> vec4<f32> {
    var uv = uv0 + cam * CAM_RIGID;
    var d = 0.5;
    if (has_depth > 0.0) {
        // Find where each flat layer lands: nearer layers magnify a little more around the
        // focal point and shift further against the camera.
        for (var i = 0; i < 3; i++) {
            d = textureSampleLevel(dt, ds, uv, 0.0).r;
            uv = f + (uv0 - f) * (1.0 - dolly * d) + cam * (CAM_RIGID + CAM_PARALLAX * (d - 0.4));
        }
    }
    let time = params.x;
    var m = vec4<f32>(0.0);
    if (has_depth > 0.0) {
        m = textureSampleLevel(ft, fs, uv, 0.0);
        // Clouds drift a touch: a tiny, slow, broad displacement inside the sky only.
        let drift = vec2<f32>(sin(uv.y * 9.0 + time * 0.21) * 0.0007, cos(uv.x * 7.0 + time * 0.17) * 0.0003);
        uv += drift * m.a;
    }
    var c = sample_bicubic(t, s, uv);
    let soften = smoothstep(2.5, 4.5, up);
    if (soften > 0.0) {
        c = mix(c, sample_soft(t, s, uv, up), soften);
    }
    if (has_depth > 0.0) {
        // Fire flickers: only the flames' bright parts, and with no overall brightening, so
        // nothing gets tinted.
        let n = sin(time * 7.0 + uv.x * 60.0 + uv.y * 45.0) * 0.5
            + sin(time * 11.3 - uv.y * 80.0) * 0.3
            + sin(time * 3.1 + uv.x * 23.0) * 0.2;
        let flame = m.r * smoothstep(0.45, 0.85, dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)));
        c *= 1.0 + flame * 0.1 * n;
        // Lights and stars twinkle, each at its own pace (only their bright parts).
        let lum = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        let tw = sin(time * (1.6 + hash2(floor(uv * 300.0)).y * 2.0) + hash2(floor(uv * 300.0)).x * 6.283);
        c *= 1.0 + m.g * smoothstep(0.2, 0.6, lum) * (0.15 + 0.3 * tw);
        // Water catches a slow, broad moving sheen.
        c *= 1.0 + m.b * 0.08 * max(sin(uv.x * 60.0 - time * 0.9 + sin(uv.y * 40.0 + time * 0.3) * 2.0), 0.0);
    }
    // Light breathes onto the subject: near parts brighten slightly, far parts recede.
    if (has_depth > 0.0) {
        c *= 1.0 + RELIGHT * br * (d - 0.45);
    }
    return vec4<f32>(c, d);
}

fn magnification(screen: vec2<f32>, tex: vec2<f32>, z: f32, f: vec2<f32>) -> f32 {
    return screen.x / (tex.x * layout_params(screen, tex, f).z) * z;
}

// The image fades out over its lower part into the scene below, and is absent past it.
fn hero_mask(huv: vec2<f32>) -> f32 {
    return mix(1.0 - smoothstep(0.7, 1.0, huv.y), 1.0, backdrop.x);
}

fn srgb_alpha(a: f32) -> f32 {
    return 1.0 - pow(1.0 - clamp(a, 0.0, 1.0), 2.2);
}

fn hash2(p: vec2<f32>) -> vec2<f32> {
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453);
}

fn ribbon(uv: vec2<f32>, t: f32, freq: f32, speed: f32, phase: f32, base: f32, width: f32) -> f32 {
    let y = base
        + 0.10 * sin(uv.x * freq + t * speed + phase)
        + 0.04 * sin(uv.x * freq * 2.7 - t * speed * 0.6 + phase * 1.9);
    let d = (uv.y - y) / width;
    return exp(-d * d);
}

// Shafts of light fanning down and inward from just above a top corner.
fn corner_beams(uv: vec2<f32>, aspect: f32, t: f32, seed: f32, right: bool) -> f32 {
    let side = select(1.0, -1.0, right);
    let cx = select(-0.06, 1.06, right) * aspect;
    let q = vec2<f32>(uv.x * aspect, uv.y) - vec2<f32>(cx, -0.14);
    let ang = atan2(q.x * side, q.y) + 0.07 * sin(t * 0.17 + seed * 9.0); // 0 = straight down
    let fan = smoothstep(-0.05, 0.3, ang) * smoothstep(1.4, 0.95, ang);
    // Two or three broad, soft shafts rather than many thin ones.
    let f = 3.0 + seed * 1.5;
    let s1 = pow(0.5 + 0.5 * sin(ang * f + seed * 17.0), 5.0);
    let breathe = 0.7 + 0.3 * sin(t * 0.15 + seed * 3.0);
    let len = length(q);
    let fall = exp(-len * 1.6) * smoothstep(0.05, 0.35, len);
    return s1 * breathe * fan * fall;
}

// One candidate particle per grid cell; `p` is in cell units. Returns the offset from the
// particle (xy) and two randoms (zw), or z < 0 for an empty cell.
fn cell_particle(p: vec2<f32>, salt: f32, density: f32) -> vec4<f32> {
    let cell = floor(p);
    let h = hash2(cell + vec2<f32>(salt * 37.0, salt * 91.0));
    if (h.x > density) {
        return vec4<f32>(0.0, 0.0, -1.0, 0.0);
    }
    let off = (hash2(cell * 1.7 + salt * 3.1) - 0.5) * 0.6;
    let f = fract(p) - 0.5 - off;
    return vec4<f32>(f, h.y, h.x / density);
}

fn atmosphere(kind: i32, uv: vec2<f32>, aspect: f32, t: f32, seed: f32) -> f32 {
    // One sparse layer per style: a handful of particles on screen, never a field of them.
    let base = vec2<f32>(uv.x * aspect, uv.y);
    var acc = 0.0;
    if (kind == 1) {
        // Dust motes: a few soft specks wandering lazily, now and then catching the light.
        var p = base * 4.0 + vec2<f32>(t * 0.03, -t * 0.018);
        p += vec2<f32>(sin(p.y * 0.7 + t * 0.3), cos(p.x * 0.6 + t * 0.25)) * 0.15;
        let c = cell_particle(p, seed, 0.12);
        if (c.z >= 0.0) {
            let r = 0.03 + 0.015 * c.z;
            let glint = pow(0.5 + 0.5 * sin(t * (0.5 + c.w) + c.z * 30.0), 4.0);
            acc += exp(-dot(c.xy, c.xy) / (r * r)) * (0.08 + 0.3 * glint);
        }
    } else if (kind == 2) {
        // Snow / ash: slow, sparse, swaying down.
        var p = base * 5.0;
        p.y -= t * 0.03 * 5.0;
        p.x += sin(p.y * 0.5 + t * 0.5) * 0.3;
        let c = cell_particle(p, seed, 0.14);
        if (c.z >= 0.0) {
            let r = 0.035 + 0.01 * c.z;
            acc += exp(-dot(c.xy, c.xy) / (r * r)) * (0.18 + 0.1 * c.z);
        }
    } else if (kind == 4) {
        // Bokeh: a few big soft out-of-focus discs drifting through.
        let p = base * 2.2 + vec2<f32>(t * 0.015, -t * 0.01) + seed * 7.0;
        let c = cell_particle(p, seed, 0.25);
        if (c.z >= 0.0) {
            let r = 0.16 + 0.08 * c.z;
            let d = length(c.xy);
            let disc = smoothstep(r, r * 0.55, d) * (0.75 + 0.25 * smoothstep(r * 0.4, r * 0.85, d));
            acc += disc * (0.022 + 0.012 * sin(t * 0.5 + c.w * 12.0));
        }
    } else if (kind == 5) {
        // Sparks: rare, small, rising, bright enough to bloom.
        var p = base * 6.0;
        p.y += t * 0.08 * 6.0;
        p.x += sin(p.y * 0.8 + t) * 0.2;
        let c = cell_particle(p, seed, 0.07);
        if (c.z >= 0.0) {
            let q = c.xy * vec2<f32>(1.0, 0.4);
            let twinkle = 0.5 + 0.5 * sin(t * (2.0 + c.z * 3.0) + c.w * 40.0);
            acc += exp(-dot(q, q) / 0.0005) * twinkle * 0.8 * smoothstep(0.0, 0.7, uv.y);
        }
    }
    return acc;
}

@fragment
fn fragment(in: FragIn) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = params.x;
    let c = tint.rgb;
    let screen = max(zoom.zw, vec2<f32>(1.0));
    let aspect = screen.x / screen.y;
    let seed = style.z;
    // The accent at full brightness, for light sources.
    let light = c / max(max(c.r, max(c.g, c.b)), 0.001);

    // Ambient: deep blue-black with soft accent light and drifting ribbons (no hero art).
    var ambient = mix(c * 0.05 + vec3<f32>(0.004, 0.006, 0.010), INK, smoothstep(0.0, 1.0, uv.y));
    ambient += c * 0.18 * exp(-length((uv - vec2<f32>(0.72, 0.28)) * vec2<f32>(1.4, 2.0)) * 2.6);
    let r = ribbon(uv, t, 4.2, 0.30, 0.0, 0.52, 0.030) * 0.6
        + ribbon(uv, t, 3.1, -0.22, 2.1, 0.60, 0.055) * 0.35
        + ribbon(uv, t, 5.6, 0.18, 4.4, 0.46, 0.016) * 0.5;
    ambient += (c * 0.6 + vec3<f32>(0.08)) * r * 0.10;

    // Hero art, as a slowly moving 2.5D photo.
    var col = ambient;
    var near = 0.0;
    var has_hero = 0.0;
    if (params.z > 0.0) {
        let huv = hero_uv(uv, screen, sizes.xy, zoom.x, focal.xy);
        let m = hero_mask(huv);
        if (m > 0.0) {
            let h = hero_sample(hero_a, hero_a_s, depth_a, depth_a_s, fxmap_a, fxmap_a_s, huv, look.z, magnification(screen, sizes.xy, zoom.x, focal.xy), focal.xy, breath.x, motion.xy, breath.z);
            let w = params.z * (1.0 - params.y) * m;
            col = mix(col, h.rgb, w);
            near = mix(near, h.a, w);
            has_hero = max(has_hero, w);
        }
    }
    if (params.w > 0.0) {
        let huv = hero_uv(uv, screen, sizes.zw, zoom.y, focal.zw);
        let m = hero_mask(huv);
        if (m > 0.0) {
            let h = hero_sample(hero_b, hero_b_s, depth_b, depth_b_s, fxmap_b, fxmap_b_s, huv, look.w, magnification(screen, sizes.zw, zoom.y, focal.zw), focal.zw, breath.y, motion.zw, breath.w);
            let w = params.w * params.y * m;
            col = mix(col, h.rgb, w);
            near = mix(near, h.a, w);
            has_hero = max(has_hero, w);
        }
    }
    // Atmospheric perspective: far parts take on a little of the scene's colour.
    col = mix(col, light * 0.12 + col * 0.8, (1.0 - near) * 0.25 * has_hero);

    // Shade only where the UI sits: left (info), bottom (shelf), top (tabs).
    let left_s = clamp(select(mix(0.92, 0.55, uv.x / 0.32), mix(0.55, 0.0, (uv.x - 0.32) / 0.30), uv.x > 0.32), 0.0, 1.0);
    let by = 1.0 - uv.y;
    let bottom_s = clamp(select(mix(1.0, 0.85, by / 0.26), mix(0.85, 0.0, (by - 0.26) / 0.32), by > 0.26), 0.0, 1.0);
    let top_s = clamp(mix(0.7, 0.0, uv.y / 0.18), 0.0, 1.0);
    // Gradients are specified like CSS (sRGB alpha); convert to equivalent linear-space
    // coverage so the darkening looks as strong as designed.
    // (The grid darkens evenly on its own instead.)
    let ui = 1.0 - backdrop.x;
    col = mix(col, INK, srgb_alpha(left_s) * ui);
    col = mix(col, INK, srgb_alpha(bottom_s) * ui);
    col = mix(col, INK, srgb_alpha(top_s) * ui);

    // Gentle vignette.
    let v = smoothstep(1.3, 0.4, length((uv - 0.5) * vec2<f32>(1.2, 1.0)));
    col *= mix(0.7, 1.0, v);
    // The grid's backdrop is dimmed here, before bloom, so it stays deep instead of hazy.
    col *= 1.0 - 0.6 * backdrop.x;

    let amount = style.y;
    // Light beams from the top corners; foreground subjects block some of them.
    let beam_col = mix(light, vec3<f32>(1.0, 0.9, 0.75), style.w);
    var b = 0.0;
    if (beams.x > 0.0) {
        b += corner_beams(uv, aspect, t, seed, false) * beams.x;
    }
    if (beams.y > 0.0) {
        b += corner_beams(uv, aspect, t, fract(seed + 0.37), true) * beams.y;
    }
    b *= (1.0 - 0.6 * near * has_hero) * (1.0 - 0.7 * srgb_alpha(bottom_s)) * (1.0 - 0.35 * srgb_alpha(left_s)) * (1.0 + 0.6 * fx.y);
    col += beam_col * b * 0.3 * amount;

    // Atmosphere particles, kept sparse and soft.
    let kind = i32(style.x + 0.5);
    if (kind > 0) {
        let p = atmosphere(kind, uv, aspect, t, seed);
        var pc = mix(light, vec3<f32>(1.0, 0.95, 0.9), 0.6);
        if (kind == 5) { pc = mix(light, vec3<f32>(1.0, 0.7, 0.35), 0.5); }
        if (kind == 4) { pc = mix(light, vec3<f32>(1.0), 0.25); }
        if (kind == 2) { pc = mix(light, vec3<f32>(0.9, 0.93, 1.0), 0.7); }
        col += pc * p * amount * (1.0 - 0.75 * srgb_alpha(left_s));
    }

    // Launch: a burst of light from the selected cover that bloom blows out.
    if (fx.x > 0.0) {
        let d = length((uv - fx.zw) * vec2<f32>(aspect, 1.0));
        col += (light * 0.8 + vec3<f32>(0.4)) * fx.x * (2.4 * exp(-d * 2.5) + 0.22);
    }

    return vec4<f32>(col, 1.0);
}
