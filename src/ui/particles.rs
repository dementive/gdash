//! Particles that come out of the hero image: embers rising off fire, glints flashing on
//! lights and water, stars twinkling in a night sky, leaves drifting down from trees, dust
//! hanging around lights. Spawn points come from the scene analysis (`scene.rs`); each
//! particle is pinned to the depth card it was born on, so it moves with the picture.
//!
//! All particles are one dynamic mesh drawn in the world, above the background, so they get
//! bloom. Premultiplied blending lets glowing particles add light (alpha 0) while leaves cover
//! what's behind them.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, BlendState, PrimitiveTopology, RenderPipelineDescriptor, SpecializedMeshPipelineError};
use bevy::shader::{Shader, ShaderRef};
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPipeline, Material2dPlugin};
use bevy::window::PrimaryWindow;

use super::scene::{EmitKind, Emitter};

pub const SHADER: Handle<Shader> = bevy::asset::uuid_handle!("2b7c4e0a-8d51-4f0e-9a6b-3c1d5e7f9a20");

// Mirrors of the background shader's layout constants (background.wgsl).
const CAM_RIGID: f32 = 0.004;
const CAM_PARALLAX: f32 = 0.004;
const MIN_COVER: f32 = 0.62;
const OVERSCAN: f32 = 1.045;

/// At most this many particles at once, whatever the image.
const MAX_PARTICLES: usize = 90;

#[derive(AsBindGroup, Asset, TypePath, Debug, Clone, Default)]
pub struct ParticleMaterial {}

impl Material2d for ParticleMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER)
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
    fn specialize(
        _pipeline: &Material2dPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(target) = descriptor.fragment.as_mut().and_then(|f| f.targets.get_mut(0)).and_then(|t| t.as_mut()) {
            target.blend = Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        }
        Ok(())
    }
}

/// Where each hero image currently sits on screen (written by `update_background`).
#[derive(Resource, Default)]
pub struct HeroLayout {
    pub slots: Vec<SlotView>,
}

#[derive(Clone)]
pub struct SlotView {
    pub id: String,
    pub tex: Vec2,
    pub zoom: f32,
    pub focal: Vec2,
    pub dolly: f32,
    pub cam: Vec2,
    /// How visible the image is (cross-fade).
    pub weight: f32,
    /// 0 showcase layout, 1 full-screen backdrop (see background.wgsl `layout_params`).
    pub fill: f32,
    pub emitters: Arc<Vec<Emitter>>,
}

impl SlotView {
    /// Where image point `p` (on a card at depth `d`) is on screen, in screen UV. Inverts the
    /// background shader's layout and parallax.
    fn to_screen(&self, p: Vec2, d: f32, screen: Vec2) -> Vec2 {
        let k = CAM_RIGID + CAM_PARALLAX * (d - 0.4);
        let uv0 = self.focal + (p - self.focal - self.cam * k) / (1.0 - self.dolly * d).max(0.5);
        let base = self.focal + (uv0 - self.focal) * self.zoom;
        let f = self.focal;
        let scale1 = (screen.x / self.tex.x).max(screen.y * MIN_COVER / self.tex.y) * OVERSCAN;
        let shown1 = screen / (self.tex * scale1);
        let cx1 = f.x.clamp(shown1.x * 0.5, 1.0 - shown1.x * 0.5);
        let y01 = if shown1.y < 1.0 { (f.y - shown1.y * 0.5).clamp(0.0, 1.0 - shown1.y) } else { 0.0 };
        let scale2 = (screen.x / self.tex.x).max(screen.y / self.tex.y) * OVERSCAN;
        let shown2 = screen / (self.tex * scale2);
        let cx2 = f.x.clamp(shown2.x * 0.5, 1.0 - shown2.x * 0.5);
        let y02 = f.y.clamp(shown2.y * 0.5, 1.0 - shown2.y * 0.5) - 0.5 * shown2.y;
        let t = self.fill;
        let (cx, y0, shown) = (cx1 + (cx2 - cx1) * t, y01 + (y02 - y01) * t, shown1.lerp(shown2, t));
        Vec2::new((base.x - cx) / shown.x + 0.5, (base.y - y0) / shown.y)
    }
}

struct Particle {
    slot: String,
    kind: EmitKind,
    /// Birth place in the image and its card depth.
    uv: Vec2,
    depth: f32,
    /// Travel since birth, in screen points (y up).
    off: Vec2,
    vel: Vec2,
    age: f32,
    life: f32,
    size: f32,
    color: Vec3,
    seed: f32,
}

#[derive(Resource)]
pub struct Particles {
    list: Vec<Particle>,
    rng: u64,
    /// Spawn credit per kind for the current image.
    credit: [f32; 5],
    mesh: Handle<Mesh>,
}

impl Particles {
    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[derive(Component)]
pub struct ParticleNode;

pub fn register(app: &mut App) {
    app.add_plugins(Material2dPlugin::<ParticleMaterial>::default());
    let shader = Shader::from_wgsl(include_str!("shaders/particles.wgsl"), "gdash/particles.wgsl");
    let _ = app.world_mut().resource_mut::<Assets<Shader>>().insert(SHADER.id(), shader);
    app.init_resource::<HeroLayout>();
}

pub fn spawn(commands: &mut Commands, meshes: &mut Assets<Mesh>, mats: &mut Assets<ParticleMaterial>) {
    let mesh = meshes.add(build_mesh(&[], &[], &[], &[]));
    commands.insert_resource(Particles { list: Vec::new(), rng: 0x9E37_79B9_7F4A_7C15, credit: [0.0; 5], mesh: mesh.clone() });
    commands.spawn((
        ParticleNode,
        Mesh2d(mesh),
        MeshMaterial2d(mats.add(ParticleMaterial::default())),
        Transform::from_xyz(0.0, 0.0, 5.0),
        NoFrustumCulling,
    ));
}

fn kind_index(k: EmitKind) -> usize {
    match k {
        EmitKind::Ember => 0,
        EmitKind::Glint => 1,
        EmitKind::Star => 2,
        EmitKind::Leaf => 3,
        EmitKind::Mote => 4,
    }
}

/// Births per second for a kind, from how many emitters of it the image has.
fn rate(kind: EmitKind, count: usize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let c = count as f32;
    match kind {
        // A lone hot spot is more likely a stray orange highlight than a fire.
        EmitKind::Ember if count < 3 => 0.0,
        EmitKind::Ember => (0.4 * c).min(5.0),
        EmitKind::Glint => (0.15 + 0.04 * c).min(0.5),
        EmitKind::Star => (0.6 + 0.05 * c).min(1.6),
        EmitKind::Leaf => (0.12 + 0.03 * c).min(0.35),
        EmitKind::Mote => (0.3 + 0.05 * c).min(0.6),
    }
}

pub fn update_particles(
    time: Res<Time>,
    layout: Res<HeroLayout>,
    mut parts: ResMut<Particles>,
    mut meshes: ResMut<Assets<Mesh>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let dt = time.delta_secs().min(0.1);
    let screen = windows.single().map(|w| Vec2::new(w.width(), w.height())).unwrap_or(Vec2::new(1600.0, 900.0));
    let unit = screen.y / 1080.0;
    let parts = &mut *parts;

    // Births, from the image being faded in.
    if let Some(slot) = layout.slots.last().filter(|s| !s.emitters.is_empty()) {
        for kind in [EmitKind::Ember, EmitKind::Glint, EmitKind::Star, EmitKind::Leaf, EmitKind::Mote] {
            let pool: Vec<&Emitter> = slot.emitters.iter().filter(|e| e.kind == kind).collect();
            let ki = kind_index(kind);
            parts.credit[ki] = (parts.credit[ki] + rate(kind, pool.len()) * dt * slot.weight).min(3.0);
            while parts.credit[ki] >= 1.0 && parts.list.len() < MAX_PARTICLES {
                parts.credit[ki] -= 1.0;
                // Stronger emitters give more.
                let total: f32 = pool.iter().map(|e| e.strength).sum();
                let mut pick = parts.rand() * total;
                let e = pool.iter().find(|e| {
                    pick -= e.strength;
                    pick <= 0.0
                }).unwrap_or(&pool[0]);
                let (r1, r2, r3, r4) = (parts.rand(), parts.rand(), parts.rand(), parts.rand());
                let jitter = Vec2::new(r1 - 0.5, r2 - 0.5);
                let p = match kind {
                    EmitKind::Ember => Particle {
                        slot: slot.id.clone(),
                        kind,
                        uv: e.uv + jitter * 0.012,
                        depth: e.depth,
                        off: Vec2::ZERO,
                        vel: Vec2::new((r3 - 0.5) * 30.0, 45.0 + 55.0 * r4) * unit,
                        age: 0.0,
                        life: 2.2 + 2.0 * r3,
                        size: (2.0 + 1.8 * r4) * unit,
                        color: (e.color * 0.4 + Vec3::new(1.0, 0.42, 0.1) * 0.6) * (2.5 + 2.0 * r1),
                        seed: r2,
                    },
                    EmitKind::Glint => Particle {
                        slot: slot.id.clone(),
                        kind,
                        uv: e.uv + jitter * 0.004,
                        depth: e.depth,
                        off: Vec2::ZERO,
                        vel: Vec2::ZERO,
                        age: 0.0,
                        life: 0.8 + 0.5 * r3,
                        size: (5.0 + 5.0 * r4 * e.strength) * unit,
                        color: (e.color * 0.5 + Vec3::splat(0.5)) * 3.0,
                        seed: r2,
                    },
                    EmitKind::Star => Particle {
                        slot: slot.id.clone(),
                        kind,
                        uv: e.uv + jitter * 0.002,
                        depth: e.depth,
                        off: Vec2::ZERO,
                        vel: Vec2::ZERO,
                        age: 0.0,
                        life: 1.2 + 1.2 * r3,
                        size: (2.5 + 2.5 * r4) * unit,
                        color: (e.color * 0.3 + Vec3::new(0.8, 0.88, 1.0) * 0.7) * 2.2,
                        seed: r2,
                    },
                    EmitKind::Leaf => Particle {
                        slot: slot.id.clone(),
                        kind,
                        uv: e.uv + jitter * 0.03,
                        depth: e.depth,
                        off: Vec2::ZERO,
                        vel: Vec2::new((r3 - 0.5) * 20.0, -(18.0 + 14.0 * r4)) * unit,
                        age: 0.0,
                        life: 6.0 + 3.0 * r3,
                        size: (5.0 + 3.0 * r4) * unit,
                        // Sunlit leaf: the canopy's own colour, a little brighter.
                        color: (e.color * 0.75 + Vec3::splat(0.1)).min(Vec3::ONE),
                        seed: r2,
                    },
                    EmitKind::Mote => Particle {
                        slot: slot.id.clone(),
                        kind,
                        uv: e.uv + jitter * 0.06,
                        depth: e.depth,
                        off: Vec2::ZERO,
                        vel: Vec2::new(r3 - 0.5, r4 - 0.3) * 10.0 * unit,
                        age: 0.0,
                        life: 4.0 + 3.0 * r3,
                        size: (1.6 + 1.6 * r4) * unit,
                        color: (e.color * 0.5 + Vec3::splat(0.5)) * 1.2,
                        seed: r2,
                    },
                };
                parts.list.push(p);
            }
        }
    }

    // Movement and death.
    parts.list.retain_mut(|p| {
        p.age += dt;
        let wob = (p.age * (1.3 + p.seed) + p.seed * 20.0).sin();
        match p.kind {
            EmitKind::Ember => {
                p.vel.y *= 1.0 - 0.25 * dt;
                p.off += (p.vel + Vec2::new(wob * 14.0 * unit, 0.0)) * dt;
            }
            EmitKind::Leaf => p.off += (p.vel + Vec2::new(wob * 28.0 * unit, 0.0)) * dt,
            EmitKind::Mote => p.off += (p.vel + Vec2::new(wob, (p.age * 0.9 + p.seed * 7.0).cos()) * 4.0 * unit) * dt,
            EmitKind::Glint | EmitKind::Star => {}
        }
        p.age < p.life && layout.slots.iter().any(|s| s.id == p.slot)
    });

    // Geometry.
    let (mut pos, mut uv, mut col, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for p in &parts.list {
        let Some(slot) = layout.slots.iter().find(|s| s.id == p.slot) else { continue };
        let at = slot.to_screen(p.uv, p.depth, screen);
        // The image fades out over its lower part; particles do too.
        let fade_low = 1.0 - smooth(0.7, 0.95, at.y) * (1.0 - slot.fill);
        let shade = ui_shade(at) + (1.0 - ui_shade(at)) * slot.fill;
        let t = p.age / p.life;
        let (shape, mut a, axis, half) = match p.kind {
            EmitKind::Ember => {
                let flick = 0.75 + 0.25 * (p.age * 17.0 + p.seed * 40.0).sin();
                let dir = (p.vel + Vec2::new(0.0, 1.0)).normalize_or_zero();
                (0.0, smooth(0.0, 0.08, t) * (1.0 - smooth(0.4, 1.0, t)) * flick, dir, Vec2::new(p.size * 2.4, p.size))
            }
            EmitKind::Glint => {
                let env = (t * std::f32::consts::PI).sin().powf(1.5);
                let s = p.size * (0.5 + 0.5 * env);
                (1.0, env, Vec2::from_angle(p.seed * 0.6 + t * 0.4), Vec2::splat(s * 3.0))
            }
            EmitKind::Star => {
                let env = (t * std::f32::consts::PI).sin();
                (1.0, env * 0.8, Vec2::X, Vec2::splat(p.size * 2.5))
            }
            EmitKind::Leaf => {
                let spin = p.age * (0.8 + p.seed) + p.seed * 6.0;
                let flip = 0.35 + 0.65 * (spin * 1.7).cos().abs();
                (2.0, smooth(0.0, 0.1, t) * (1.0 - smooth(0.75, 1.0, t)) * 0.85, Vec2::from_angle(spin), Vec2::new(p.size, p.size * 0.45 * flip))
            }
            EmitKind::Mote => (0.0, (t * std::f32::consts::PI).sin() * 0.35, Vec2::X, Vec2::splat(p.size * 1.5)),
        };
        a *= slot.weight * fade_low * shade;
        if a <= 0.003 {
            continue;
        }
        let center = Vec2::new((at.x - 0.5) * screen.x, (0.5 - at.y) * screen.y) + p.off;
        let side = axis.perp();
        let base = pos.len() as u32;
        for (cx, cy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let v = center + axis * cx * half.x + side * cy * half.y;
            pos.push([v.x, v.y, 0.0]);
            uv.push([cx + shape * 10.0, cy]);
            col.push([p.color.x, p.color.y, p.color.z, a]);
        }
        idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    if let Some(mut mesh) = meshes.get_mut(&parts.mesh) {
        *mesh = build_mesh(&pos, &uv, &col, &idx);
    }
}

fn build_mesh(pos: &[[f32; 3]], uv: &[[f32; 2]], col: &[[f32; 4]], idx: &[u32]) -> Mesh {
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    if pos.is_empty() {
        // An invisible triangle keeps the pipeline valid.
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32, 0.0, 0.0]; 3]);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; 3]);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 3]);
        m.insert_indices(Indices::U32(vec![0, 1, 2]));
        return m;
    }
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos.to_vec());
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv.to_vec());
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col.to_vec());
    m.insert_indices(Indices::U32(idx.to_vec()));
    m
}

/// How much of a particle shows through the darkening behind the UI (matches background.wgsl).
fn ui_shade(uv: Vec2) -> f32 {
    let srgb_alpha = |a: f32| 1.0 - (1.0 - a.clamp(0.0, 1.0)).powf(2.2);
    let left = if uv.x > 0.32 { 0.55 * (1.0 - (uv.x - 0.32) / 0.30) } else { 0.92 + (0.55 - 0.92) * uv.x / 0.32 };
    let by = 1.0 - uv.y;
    let bottom = if by > 0.26 { 0.85 * (1.0 - (by - 0.26) / 0.32) } else { 1.0 - 0.15 * by / 0.26 };
    (1.0 - 0.9 * srgb_alpha(left)) * (1.0 - srgb_alpha(bottom))
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_mapping_matches_the_layout() {
        // A 3:1 image on a 16:9 screen, no motion: full width, top-anchored.
        let s = SlotView {
            id: String::new(),
            tex: Vec2::new(3840.0, 1280.0),
            zoom: 1.0,
            focal: Vec2::new(0.5, 0.4),
            dolly: 0.0,
            cam: Vec2::ZERO,
            weight: 1.0,
            fill: 0.0,
            emitters: Arc::new(Vec::new()),
        };
        let screen = Vec2::new(1920.0, 1080.0);
        let c = s.to_screen(Vec2::new(0.5, 0.0), 0.4, screen);
        assert!((c.x - 0.5).abs() < 1e-4 && c.y.abs() < 1e-4, "{c}");
        // Moving right in the image moves right on screen, magnified by the overscan.
        let r = s.to_screen(Vec2::new(0.6, 0.0), 0.4, screen);
        assert!(r.x > 0.6 && r.x < 0.62, "{r}");
    }
}
