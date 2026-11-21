//! Opt-in, compiled particle behavior. The ordinary engine still owns the
//! particle, its sprite set, interpolation, rendering and total count limits.
use super::{Particle, ParticleLevel, ParticleOptions};
use mod_api::{
    BlockState, LightLayer, ModParticle, ParticleFacing, ParticleLevelView, ParticleProvider,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{collections::BTreeMap, fmt::Debug, sync::Arc, sync::OnceLock};

/// Requests made by one particle tick, submitted through normal spawn limits.
#[derive(Default)]
pub struct ParticleUpdate {
    /// An index in this particle's declared sprite set. Out-of-range is clamped.
    pub frame: Option<usize>,
    pub spawned: Vec<(ParticleOptions, [f64; 3], [f64; 3])>,
}

/// Owned behavior for a compiled native particle. Cloning a particle clones its
/// controller state. Do not retain world references; inspect `level` each tick.
pub trait ParticleController: Clone + Debug + Send + Sync + 'static {
    /// Call `particle.advance_age()` for ordinary lifetime/interpolation, or
    /// explicitly maintain those fields for an overridden source tick.
    fn tick(&mut self, particle: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate;
}

pub(super) trait Controller: Debug + Send + Sync {
    fn tick(&mut self, particle: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate;
    fn clone_box(&self) -> Box<dyn Controller>;
}
impl<T: ParticleController> Controller for T {
    fn tick(&mut self, particle: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate {
        ParticleController::tick(self, particle, level)
    }
    fn clone_box(&self) -> Box<dyn Controller> {
        Box::new(self.clone())
    }
}
impl Clone for Box<dyn Controller> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// The mods' particle providers by type name: set once at start, before the resources first load,
/// and read by every engine built after.
static REGISTERED: OnceLock<BTreeMap<String, Arc<dyn ParticleProvider>>> = OnceLock::new();

/// Sets the mods' particle providers (`register_particle_providers`), once, before
/// the resources first load; each engine then resolves its sprite sets' providers at every load.
/// False when already set.
pub fn register_particle_providers(providers: BTreeMap<String, Arc<dyn ParticleProvider>>) -> bool {
    REGISTERED.set(providers).is_ok()
}

/// What [`register_particle_providers`] set; none before.
pub(super) fn registered() -> &'static BTreeMap<String, Arc<dyn ParticleProvider>> {
    static NONE: BTreeMap<String, Arc<dyn ParticleProvider>> = BTreeMap::new();
    REGISTERED.get().unwrap_or(&NONE)
}

/// How far from the origin a provider may move a particle: the world border's limit. Past it, or
/// not a number, the particle expires rather than reach the renderer.
const LIMIT: f64 = 3.0e7;

/// How large a roll a provider may leave, in radians: half of `f32::MAX`, so the difference of
/// two accepted rolls, which the renderer interpolates, is a finite number. Past it, or not a
/// number, the particle expires.
const ROLL_LIMIT: f32 = f32::MAX / 2.0;

/// A particle of a mod's code-defined type. It owns the particle's behaviour for the engine: age
/// and expiry stay the engine's, the motion is the provider's, called once per type per tick with
/// every live particle of the type ([`Batch::run`]). Holds the provider the type resolved to when
/// its sprite sets loaded, so a tick dispatches straight to it.
pub(super) struct Provided {
    provider: Arc<dyn ParticleProvider>,
    /// The provider's dense slot among the registered ones, which its batch is filled by.
    slot: u32,
    /// `ModParticle::state`, kept from one tick to the next.
    state: [f64; 4],
    /// The frame of the sprite set drawn, as the provider last saw it.
    frame: u32,
    /// The engine aged the particle this tick and the provider has not run on it yet.
    pending: bool,
    /// How the quad turns toward the viewer, as the provider last set it.
    facing: Facing,
    /// The engine's count of live provider particles, which this one is part of until it drops.
    live: Arc<AtomicUsize>,
}
/// A provider particle's facing, as the renderer keeps it (`ParticleFacing`, the quaternion
/// already normalised).
#[derive(Clone, Copy, Debug)]
enum Facing {
    Camera,
    AroundY,
    Fixed(glam::Quat),
}
impl Clone for Provided {
    fn clone(&self) -> Self {
        self.live.fetch_add(1, Ordering::Relaxed);
        Self {
            provider: self.provider.clone(),
            slot: self.slot,
            state: self.state,
            frame: self.frame,
            pending: self.pending,
            facing: self.facing,
            live: self.live.clone(),
        }
    }
}
impl Drop for Provided {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::Relaxed);
    }
}
impl Debug for Provided {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provided")
            .field("state", &self.state)
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}
impl Provided {
    pub(super) fn new(
        provider: Arc<dyn ParticleProvider>,
        slot: u32,
        frame: u32,
        live: &Arc<AtomicUsize>,
    ) -> Self {
        live.fetch_add(1, Ordering::Relaxed);
        Self {
            provider,
            slot,
            state: [0.0; 4],
            frame,
            pending: false,
            facing: Facing::Camera,
            live: live.clone(),
        }
    }
    /// The quad's rotation at `partial` of a tick (`SingleQuadParticle.extract`): the facing's
    /// quaternion (`LOOKAT_XYZ` the camera's, `LOOKAT_Y` its y and w alone, as vanilla sets it,
    /// or the world-fixed one), then `rotateZ` of `Mth.lerp(partial, oRoll, roll)` when the
    /// particle's roll is not zero. All of it in JOML's f32 maths.
    pub(super) fn rotation(
        &self,
        camera: glam::Quat,
        o_roll: f32,
        roll: f32,
        partial: f32,
    ) -> glam::Quat {
        use crate::camera::joml::Quat;
        let facing = match self.facing {
            Facing::Camera => Quat {
                x: camera.x,
                y: camera.y,
                z: camera.z,
                w: camera.w,
            },
            Facing::AroundY => Quat {
                x: 0.0,
                y: camera.y,
                z: 0.0,
                w: camera.w,
            },
            Facing::Fixed(q) => Quat {
                x: q.x,
                y: q.y,
                z: q.z,
                w: q.w,
            },
        };
        let turned = if roll == 0.0 {
            facing
        } else {
            facing.rotate_z(o_roll + partial * (roll - o_roll))
        };
        glam::Quat::from_xyzw(turned.x, turned.y, turned.z, turned.w)
    }
    /// The engine's part of the tick: age, `previous` and expiry. A particle that survives it
    /// joins this tick's batch.
    pub(super) fn age(&mut self, particle: &mut Particle) {
        self.pending = particle.advance_age();
    }
}

/// The batches of one tick, kept between ticks so native dispatch allocates nothing once warm.
/// One pass over the particles sorts them into a bucket per provider (by the dense slot the
/// provider was given when it registered); each provider then runs once on its bucket.
#[derive(Default)]
pub(super) struct Batch {
    /// The particle indices of each provider slot's batch.
    buckets: Vec<Vec<usize>>,
    /// The slots whose bucket holds a particle this pass, in the order they were first met.
    active: Vec<usize>,
    views: Vec<ModParticle>,
}
impl Batch {
    /// Sorts the particles the engine aged this tick into a bucket per provider slot. Whether
    /// any provider has a particle to tick.
    pub(super) fn gather(&mut self, particles: &[Particle]) -> bool {
        for slot in self.active.drain(..) {
            self.buckets[slot].clear();
        }
        for (i, p) in particles.iter().enumerate() {
            if let Some(provided) = p.provided.as_ref().filter(|q| q.pending) {
                let slot = provided.slot as usize;
                if self.buckets.len() <= slot {
                    self.buckets.resize_with(slot + 1, Vec::new);
                }
                if self.buckets[slot].is_empty() {
                    self.active.push(slot);
                }
                self.buckets[slot].push(i);
            }
        }
        !self.active.is_empty()
    }
    /// Calls each provider once with the particles [`Batch::gather`] found for it, then takes the
    /// results back: a particle that leaves a number past the world's limit or not finite
    /// expires.
    pub(super) fn run(
        &mut self,
        particles: &mut [Particle],
        level: &dyn ParticleLevelView,
        mut set_frame: impl FnMut(&mut Particle, usize),
    ) {
        for &slot in &self.active {
            let bucket = &self.buckets[slot];
            let provider = particles[bucket[0]]
                .provided
                .as_ref()
                .map(|provided| provided.provider.clone())
                .expect("a bucket holds provider particles");
            self.views.clear();
            for &i in bucket {
                let p = &particles[i];
                let provided = p
                    .provided
                    .as_ref()
                    .expect("a bucket holds provider particles");
                self.views.push(ModParticle {
                    position: p.position,
                    previous: p.previous,
                    velocity: p.velocity,
                    age: p.age,
                    lifetime: p.lifetime,
                    size: p.size,
                    rgba: p.rgba,
                    frame: provided.frame,
                    removed: false,
                    state: provided.state,
                    facing: p.facing(),
                    roll: p.roll,
                });
            }
            let mut batch =
                mod_api::__private::particle_batch_from_vec(std::mem::take(&mut self.views));
            provider.tick(&mut batch, level);
            self.views = mod_api::__private::particle_batch_into_vec(batch);
            if self.views.len() != bucket.len() {
                // A guest handed back a batch of another size: nothing of it can be matched to a particle.
                for &i in bucket {
                    particles[i].remove();
                }
                continue;
            }
            for (view, &i) in self.views.iter().zip(bucket) {
                if let Some(frame) = particles[i].take(view) {
                    set_frame(&mut particles[i], frame);
                }
            }
        }
    }
}
impl Particle {
    /// The facing the provider sees.
    fn facing(&self) -> ParticleFacing {
        match self.provided.as_ref().map(|q| q.facing) {
            Some(Facing::Fixed(q)) => ParticleFacing::Fixed(q.to_array()),
            Some(Facing::AroundY) => ParticleFacing::AroundY,
            _ => ParticleFacing::Camera,
        }
    }
    /// What a provider's tick left of the particle, validated as the renderer needs it.
    fn take(&mut self, view: &ModParticle) -> Option<usize> {
        let finite = |v: &[f64; 3]| v.iter().all(|c| c.abs() <= LIMIT);
        let facing = match view.facing {
            ParticleFacing::Fixed(q) => {
                let q = glam::Quat::from_array(q);
                let length_squared = q.length_squared();
                (length_squared.is_finite() && length_squared > 0.0)
                    .then(|| Facing::Fixed(q.normalize()))
            }
            ParticleFacing::AroundY => Some(Facing::AroundY),
            ParticleFacing::Camera => Some(Facing::Camera),
        };
        let provided = self.provided.as_mut()?;
        provided.pending = false;
        let Some(facing) = facing else {
            self.remove();
            return None;
        };
        if view.removed
            || !finite(&view.position)
            || !finite(&view.previous)
            || !finite(&view.velocity)
            || !view.size.is_finite()
            || !view.roll.is_finite()
            || view.roll.abs() > ROLL_LIMIT
            || !view.rgba.iter().all(|c| c.is_finite())
        {
            self.remove();
            return None;
        }
        self.position = view.position;
        self.previous = view.previous;
        self.velocity = view.velocity;
        self.lifetime = view.lifetime;
        // The declarative range of a mod particle's size (`sunrise:behaviour`) and of its colour.
        self.size = view.size.clamp(0.0, 16.0);
        self.rgba = view.rgba.map(|c| c.clamp(0.0, 1.0));
        self.roll = view.roll;
        provided.facing = facing;
        provided.state = view.state;
        let changed = view.frame != provided.frame;
        provided.frame = view.frame;
        changed.then_some(view.frame as usize)
    }
}

/// `ParticleLevelView` over the engine's level: the loaded chunks' states and stored light.
pub(super) struct ProviderLevel<'a, 'l> {
    pub(super) level: &'a ParticleLevel<'l>,
    /// Every engine state as a mod reads it, by state id.
    pub(super) states: &'a [BlockState],
}
impl ParticleLevelView for ProviderLevel<'_, '_> {
    fn block_state(&self, [x, y, z]: [i32; 3]) -> BlockState {
        // The table holds every state the world can: an id outside it is the game's bug.
        self.states[self.level.block(x, y, z).0 as usize]
    }
    fn brightness(&self, layer: LightLayer, [x, y, z]: [i32; 3]) -> u8 {
        let world = self.level.world;
        match layer {
            LightLayer::Block => world.light(x, y, z) & 15,
            // A dimension without skylight has no sky layer to read.
            LightLayer::Sky if !world.shape.sky_light => 0,
            LightLayer::Sky => world.light(x, y, z) >> 4,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::*;
    use super::ROLL_LIMIT;
    use crate::chunk::{Chunk, Light};
    use mod_api::{
        BlockState, LightLayer, ModParticle, ParticleBatch, ParticleFacing, ParticleLevelView,
        ParticleProvider,
    };
    use std::sync::Arc;
    use {
        sunrise_core::nbt::Tag,
        sunrise_world::{
            DATA_VERSION,
            collision::CachedCollisionRegistry,
            registry::{BlockRegistry, BlockStateId},
        },
    };
    #[derive(Clone, Debug)]
    struct Moving(u32);
    impl ParticleController for Moving {
        fn tick(&mut self, p: &mut Particle, _: &ParticleLevel<'_>) -> ParticleUpdate {
            if p.advance_age() {
                self.0 += 1;
                p.position[0] += f64::from(self.0);
            }
            ParticleUpdate {
                frame: Some(self.0 as usize),
                ..Default::default()
            }
        }
    }
    /// An empty world with no blocks, which a tick reads nothing of.
    pub(in crate::particles) struct Scene {
        world: World,
        facts: BlockFacts,
        collision: BlockCollisionRegistry<'static>,
    }
    impl Scene {
        pub(in crate::particles) fn new() -> Self {
            // Leaked: the collision registry borrows it for as long as the scene lives.
            let registry: &'static BlockRegistry = Box::leak(Box::new(
                BlockRegistry::from_nbt(&Tag::compound([
                    ("DataVersion", Tag::Int(DATA_VERSION)),
                    (
                        "states",
                        Tag::List(vec![Tag::String("minecraft:air".into())]),
                    ),
                    ("defaults", Tag::compound([("minecraft:air", Tag::Int(0))])),
                ]))
                .unwrap(),
            ));
            let cached = CachedCollisionRegistry::from_nbt(
                &Tag::compound([
                    ("DataVersion", Tag::Int(DATA_VERSION)),
                    (
                        "states",
                        Tag::List(vec![Tag::String("minecraft:air".into())]),
                    ),
                    ("shapes", Tag::List(vec![])),
                    ("cached_shapes", Tag::IntArray(vec![-1])),
                ]),
                registry,
            )
            .unwrap();
            let collision = BlockCollisionRegistry::new(std::sync::Arc::new(cached)).unwrap();
            let facts = BlockFacts {
                states: vec![],
                air: BlockStateId(0),
                woods: HashMap::new(),
                sign_texts: HashMap::new(),
            };
            Self {
                world: World::new(BlockStateId(0), 0),
                facts,
                collision,
            }
        }
        pub(in crate::particles) fn level(&self) -> ParticleLevel<'_> {
            ParticleLevel {
                world: &self.world,
                facts: &self.facts,
                collision: &self.collision,
                player: None,
                cached_chunk: std::cell::Cell::new(None),
            }
        }
    }
    fn engine() -> ParticleEngine {
        let mut e = ParticleEngine::new(
            &ResourceSource::indexed(Default::default()),
            &HashMap::new(),
            &HashSet::new(),
        );
        // The game's table of block states; the providers here read none of them.
        e.set_block_state_source(|| Some(Arc::new(Vec::<BlockState>::new())));
        e.mod_sets.insert(
            "example:orbit".into(),
            ModSet {
                set: 0,
                behaviour: None,
                provider: None,
                group: None,
            },
        );
        e.set_sprites.push(vec![0, 1]);
        e.rects = vec![
            Some(Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 16,
                h: 16
            });
            2
        ];
        e
    }
    #[test]
    fn controlled_particle_keeps_owned_motion_sprite_interpolation_and_expiry() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let mut p = e
            .controlled("example:orbit", [0.0; 3], true, Moving(0))
            .unwrap();
        p.lifetime = 2;
        p.rgba[3] = 0.5;
        p.velocity = [99.0; 3]; // no vanilla drift may leak into the custom tick
        let mut copy = p.clone();
        e.tick_particle(&mut p, &level);
        e.tick_particle(&mut p, &level);
        e.tick_particle(&mut copy, &level);
        assert_eq!((p.position, p.previous), ([3.0, 0.0, 0.0], [1.0, 0.0, 0.0]));
        assert_eq!(copy.position, [1.0, 0.0, 0.0]); // controller state cloned independently
        assert!(matches!(p.sprite, Sprite::Index(1))); // clamped frame
        p.size = 0.1;
        e.particles.push(p.clone());
        let mut vertices = Vec::new();
        e.render(
            DVec3::ZERO,
            glam::Quat::IDENTITY,
            0.5,
            &|_| 0,
            &mut vertices,
        );
        assert_eq!(vertices.len(), 4);
        assert!(vertices.iter().all(
            |v| v.color >> 24 == 127 && v.layer & crate::gfx::TRANSLUCENT_PARTICLE_OVERLAY != 0
        ));
        e.particles[0].set_visible(false);
        vertices.clear();
        e.render(
            DVec3::ZERO,
            glam::Quat::IDENTITY,
            0.5,
            &|_| 0,
            &mut vertices,
        );
        assert!(vertices.is_empty());
        assert_eq!(e.particles[0].age, 2);
        e.push(copy);
        e.reload(
            &ResourceSource::indexed(Default::default()),
            &HashMap::new(),
            &HashSet::new(),
        );
        assert!(e.pending.is_empty());
        vertices.clear();
        e.render(
            DVec3::ZERO,
            glam::Quat::IDENTITY,
            0.5,
            &|_| 0,
            &mut vertices,
        );
        assert!(vertices.is_empty());
        e.tick_particle(&mut p, &level);
        assert!(p.removed);
        assert_eq!(p.position, [3.0, 0.0, 0.0]);
    }
    /// Counts its ticks, so a test sees which calls a particle's expiry skipped.
    #[derive(Default)]
    struct Probe(std::sync::atomic::AtomicUsize);
    impl ParticleProvider for Probe {
        fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
            for particle in particles {
                self.one(particle);
            }
        }
    }
    impl Probe {
        fn one(&self, particle: &mut ModParticle) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            particle.position[0] = f64::from(particle.age) * 2.0;
            particle.position[1] = particle.state[0];
            particle.state[0] += 0.5;
            particle.frame = particle.age as u32;
            if particle.age == 3 {
                particle.removed = true;
                particle.position[0] = 999.0;
            }
        }
    }
    struct Broken(fn(&mut ModParticle));
    impl ParticleProvider for Broken {
        fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
            for particle in particles {
                self.0(particle);
            }
        }
    }
    /// One engine tick of a single provider particle: the age step, then the provider's batch.
    fn tick_one(e: &mut ParticleEngine, p: &mut Particle, level: &ParticleLevel<'_>) {
        e.tick_particle(p, level);
        e.tick_providers(std::slice::from_mut(p), level);
    }
    /// Counts its calls and the particles each held.
    #[derive(Default)]
    struct Calls(std::sync::Mutex<Vec<usize>>);
    impl ParticleProvider for Calls {
        fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
            self.0.lock().unwrap().push(particles.len());
        }
    }
    #[test]
    fn a_provider_runs_once_per_tick_on_all_its_particles() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let calls = Arc::new(Calls::default());
        let mut all: Vec<Particle> = (0..3).map(|_| provided(&mut e, calls.clone())).collect();
        for p in &mut all {
            p.lifetime = 20;
            e.tick_particle(p, &level);
        }
        e.tick_providers(&mut all, &level);
        assert_eq!(*calls.0.lock().unwrap(), [3]);
        // Nothing aged since: no particle is pending, so no call.
        e.tick_providers(&mut all, &level);
        assert_eq!(calls.0.lock().unwrap().len(), 1);
    }
    fn provided(e: &mut ParticleEngine, provider: Arc<dyn ParticleProvider>) -> Particle {
        e.providers.insert("example:orbit".into(), provider.clone());
        e.mod_sets.get_mut("example:orbit").unwrap().provider = Some((0, provider));
        e.make(
            &ParticleOptions::named("example:orbit"),
            [1.0; 3],
            [0.0; 3],
            5,
        )
        .unwrap()
    }
    #[test]
    fn a_providers_particle_moves_changes_frame_and_expires_by_its_tick() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let probe = Arc::new(Probe::default());
        let mut p = provided(&mut e, probe.clone());
        p.lifetime = 20;
        let mut copy = p.clone();
        tick_one(&mut e, &mut p, &level);
        assert_eq!((p.position, p.previous), ([2.0, 0.0, 1.0], [1.0; 3]));
        tick_one(&mut e, &mut p, &level);
        assert_eq!((p.position, p.previous), ([4.0, 0.5, 1.0], [2.0, 0.0, 1.0]));
        assert!(matches!(p.sprite, Sprite::Index(1))); // frame 2, clamped to the 2-sprite set
        // State is per particle: the copy starts from zero state, not the original's.
        tick_one(&mut e, &mut copy, &level);
        assert_eq!(copy.position, [2.0, 0.0, 1.0]);
        // The third tick of `p` removes it before its 20-tick lifetime, and its writes are dropped.
        tick_one(&mut e, &mut p, &level);
        assert!(p.removed);
        assert_eq!(p.position, [4.0, 0.5, 1.0]);
    }
    #[test]
    fn the_engine_expires_a_providers_particle_at_its_lifetime_without_calling_the_provider() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let probe = Arc::new(Probe::default());
        let mut p = provided(&mut e, probe.clone());
        p.lifetime = 2;
        for _ in 0..2 {
            tick_one(&mut e, &mut p, &level);
            assert!(!p.removed);
        }
        tick_one(&mut e, &mut p, &level);
        assert!(p.removed);
        assert_eq!(probe.0.load(std::sync::atomic::Ordering::Relaxed), 2);
    }
    #[test]
    fn a_provider_cannot_hand_the_renderer_a_bad_number() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let bad: [fn(&mut ModParticle); 5] = [
            |p| p.position[1] = f64::NAN,
            |p| p.position[0] = f64::INFINITY,
            |p| p.position[2] = 1.0e300,
            |p| p.velocity[0] = f64::NEG_INFINITY,
            |p| p.size = f32::NAN,
        ];
        for tick in bad {
            let mut p = provided(&mut e, Arc::new(Broken(tick)));
            tick_one(&mut e, &mut p, &level);
            assert!(p.removed);
            assert_eq!(p.position, [1.0; 3]);
        }
        let mut p = provided(
            &mut e,
            Arc::new(Broken(|p| {
                p.size = 1.0e9;
                p.rgba = [2.0, -1.0, 0.5, 9.0];
                p.frame = u32::MAX;
                p.lifetime = i32::MIN;
                p.age = i32::MAX;
            })),
        );
        tick_one(&mut e, &mut p, &level);
        assert!(!p.removed);
        assert_eq!((p.size, p.rgba), (16.0, [1.0, 0.0, 0.5, 1.0]));
        assert!(matches!(p.sprite, Sprite::Index(1)));
        assert_eq!((p.lifetime, p.age), (i32::MIN, 1)); // the age stays the engine's
        tick_one(&mut e, &mut p, &level);
        assert!(p.removed);
    }
    /// Hands back a batch of another size, as only a guest's raw bytes could.
    struct Replacing;
    impl ParticleProvider for Replacing {
        fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
            *particles = mod_api::__private::particle_batch_from_vec(Vec::new());
        }
    }
    #[test]
    fn a_batch_handed_back_at_another_size_expires_its_particles() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let mut p = provided(&mut e, Arc::new(Replacing));
        p.lifetime = 20;
        let at = p.position;
        tick_one(&mut e, &mut p, &level);
        assert!(p.removed);
        assert_eq!(p.position, at);
    }
    /// Reads the batch's two nonfinite-length quaternions.
    #[test]
    fn a_quaternion_with_an_infinite_squared_length_expires_the_particle() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let facings = [
            ParticleFacing::Fixed([1.0e30, 0.0, 0.0, 0.0]),
            ParticleFacing::Fixed([f32::NAN, 0.0, 0.0, 1.0]),
            ParticleFacing::Fixed([0.0; 4]),
        ];
        for facing in facings {
            struct Set(ParticleFacing);
            impl ParticleProvider for Set {
                fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
                    for particle in particles {
                        particle.facing = self.0;
                    }
                }
            }
            let mut p = provided(&mut e, Arc::new(Set(facing)));
            tick_one(&mut e, &mut p, &level);
            assert!(p.removed, "{facing:?}");
        }
    }
    /// A particle a provider poses on each of its ticks (`rolls[age - 1]`), drawn half a tick
    /// into the last one with the camera turned by `base`, as the renderer builds its corners.
    fn drawn(
        facing: ParticleFacing,
        rolls: &[f32],
        center: [f64; 3],
        base: glam::Quat,
    ) -> Vec<[f32; 3]> {
        struct Pose(ParticleFacing, Vec<f32>, [f64; 3]);
        impl ParticleProvider for Pose {
            fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
                for particle in particles {
                    particle.position = self.2;
                    particle.previous = self.2;
                    particle.size = 1.0;
                    particle.facing = self.0;
                    particle.roll = self.1[particle.age as usize - 1];
                }
            }
        }
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let mut p = provided(&mut e, Arc::new(Pose(facing, rolls.to_vec(), center)));
        p.lifetime = 20;
        for _ in rolls {
            tick_one(&mut e, &mut p, &level);
        }
        assert!(!p.removed);
        e.particles.push(p);
        let mut vertices = Vec::new();
        e.render(DVec3::ZERO, base, 0.5, &|_| 0, &mut vertices);
        assert_eq!(vertices.len(), 4);
        vertices.iter().map(|v| v.pos).collect()
    }
    // The expected corners below come from running JOML 1.10.9 itself: `Quaternionf.rotateZ`
    // of `Mth.lerp(0.5, oRoll, roll)` on the facing's quaternion, then `transform` of the quad's
    // corners (1,-1), (1,1), (-1,1), (-1,-1), added to the centre.
    #[test]
    fn a_providers_fixed_facing_and_roll_turn_the_drawn_quad() {
        // A unit quaternion, rolled from 0 (the first tick's) to 1: half a tick in, 0.5 radians.
        let corners = drawn(
            ParticleFacing::Fixed([-0.5, -0.5, -0.5, 0.5]),
            &[1.0],
            [0.0; 3],
            glam::Quat::IDENTITY,
        );
        assert_eq!(
            corners,
            [
                [-0.39815706, 0.0, 1.3570082],
                [1.3570082, 0.0, 0.39815706],
                [0.39815706, 0.0, -1.3570082],
                [-1.3570082, 0.0, -0.39815706],
            ]
        );
    }
    #[test]
    fn a_providers_camera_facing_rotates_the_quad_by_the_camera_then_the_interpolated_roll() {
        let corners = drawn(
            ParticleFacing::Camera,
            &[4.1, 4.5],
            [3.25, -1.5, 2.75],
            glam::Quat::from_xyzw(0.5, -0.5, 0.5, 0.5),
        );
        assert_eq!(
            corners,
            [
                [3.765367, -1.5, 1.4330349],
                [4.566965, -1.5, 3.265367],
                [2.734633, -1.5, 4.066965],
                [1.9330349, -1.5, 2.234633],
            ]
        );
    }
    #[test]
    fn a_providers_around_y_facing_takes_the_cameras_y_and_w_and_rolls_from_the_last_tick() {
        // The camera's pitch and roll components (x and z) do not reach the quad, and the
        // particle is off the camera's axes, so a yaw taken from its direction would differ.
        let camera = glam::Quat::from_xyzw(0.25, 0.5, -0.25, 0.8);
        let corners = drawn(
            ParticleFacing::AroundY,
            &[0.3, 0.9],
            [3.0, 1.0, -4.0],
            camera,
        );
        assert_eq!(
            corners,
            [
                [3.6090915, 0.7393068, -5.2494183],
                [3.1142364, 2.389978, -4.234331],
                [2.3909085, 1.2606932, -2.7505815],
                [2.8857636, -0.38997805, -3.765669],
            ]
        );
    }
    #[test]
    fn a_roll_that_returns_to_zero_draws_unrolled_as_vanilla_does() {
        // `SingleQuadParticle.extract` rolls only when the current roll is not zero.
        let camera = glam::Quat::from_xyzw(0.25, 0.5, -0.25, 0.8);
        let corners = drawn(
            ParticleFacing::AroundY,
            &[1.0, 0.0],
            [3.0, 1.0, -4.0],
            camera,
        );
        assert_eq!(
            corners,
            [
                [3.4382024, 0.0, -4.898876],
                [3.4382024, 2.0, -4.898876],
                [2.5617976, 2.0, -3.1011236],
                [2.5617976, 0.0, -3.1011236],
            ]
        );
    }
    #[test]
    fn rolls_at_the_accepted_limit_interpolate_to_finite_corners() {
        // The difference of the two extremes is `f32::MAX`, still a number.
        let corners = drawn(
            ParticleFacing::Camera,
            &[-ROLL_LIMIT, ROLL_LIMIT],
            [0.0; 3],
            glam::Quat::IDENTITY,
        );
        assert!(
            corners.iter().flatten().all(|c| c.is_finite()),
            "{corners:?}"
        );
    }
    #[test]
    fn a_roll_past_the_limit_expires_the_particle() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        for roll in [f32::MAX, -f32::MAX, ROLL_LIMIT * 1.0000001] {
            struct Set(f32);
            impl ParticleProvider for Set {
                fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
                    for particle in particles {
                        particle.roll = self.0;
                    }
                }
            }
            let mut p = provided(&mut e, Arc::new(Set(roll)));
            tick_one(&mut e, &mut p, &level);
            assert!(p.removed, "{roll}");
        }
    }
    #[test]
    fn a_second_tick_reuses_the_batch_storage() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let probe = Arc::new(Probe::default());
        let mut all: Vec<Particle> = (0..3).map(|_| provided(&mut e, probe.clone())).collect();
        let tick = |e: &mut ParticleEngine, all: &mut Vec<Particle>| {
            for p in all.iter_mut() {
                p.lifetime = 20;
                e.tick_particle(p, &level);
            }
            e.tick_providers(all, &level);
        };
        tick(&mut e, &mut all);
        let shape = |e: &ParticleEngine| {
            (
                e.batch.views.as_ptr(),
                e.batch.views.capacity(),
                e.batch.buckets.len(),
                e.batch.buckets[0].as_ptr(),
                e.batch.buckets[0].capacity(),
                e.batch.active.capacity(),
            )
        };
        let warm = shape(&e);
        tick(&mut e, &mut all);
        assert_eq!(shape(&e), warm);
        assert_eq!(probe.0.load(std::sync::atomic::Ordering::Relaxed), 6);
    }
    /// A provider type in a high slot visits only its own bucket, and an empty list visits none.
    #[test]
    fn only_the_slots_with_particles_are_visited() {
        let mut batch = native::Batch::default();
        assert!(!batch.gather(&[]));
        assert!(batch.buckets.is_empty());
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let mut p = provided(&mut e, Arc::new(Probe::default()));
        p.provided.as_mut().unwrap().slot = 40;
        p.lifetime = 20;
        e.tick_particle(&mut p, &level);
        assert!(batch.gather(std::slice::from_ref(&p)));
        assert_eq!(batch.active, [40]);
        assert!(!batch.gather(&[]));
        assert!(batch.active.is_empty());
        assert!(batch.buckets[40].is_empty());
    }
    /// The block state table is asked for only when a provider has a particle to tick.
    #[test]
    fn the_block_state_table_is_not_asked_while_no_provider_particle_is_ready() {
        static ASKED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn counting() -> Option<BlockStates> {
            ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(Arc::new(Vec::<BlockState>::new()))
        }
        let asked = || ASKED.load(std::sync::atomic::Ordering::Relaxed);
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        e.set_block_state_source(counting);
        // A provider is registered, and no particle of it lives.
        e.providers
            .insert("example:orbit".into(), Arc::new(Probe::default()));
        e.tick_providers(&mut [], &level);
        assert_eq!(asked(), 0);
        // One lives and the engine has not aged it.
        let mut p = provided(&mut e, Arc::new(Probe::default()));
        p.lifetime = 20;
        e.tick_providers(std::slice::from_mut(&mut p), &level);
        assert_eq!(asked(), 0);
        // Aged: it is ticked, and the table is asked once.
        tick_one(&mut e, &mut p, &level);
        assert_eq!(asked(), 1);
        // Gone: nothing runs.
        drop(p);
        e.tick_providers(&mut [], &level);
        assert_eq!(asked(), 1);
    }
    #[test]
    fn a_provider_does_not_run_while_the_game_has_no_block_state_table() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let probe = Arc::new(Probe::default());
        let mut p = provided(&mut e, probe.clone());
        p.lifetime = 20;
        e.set_block_state_source(|| None);
        tick_one(&mut e, &mut p, &level);
        assert_eq!(probe.0.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(p.position, [1.0; 3]);
        assert!(!p.removed);
    }
    /// A chunk whose section holds sky light 11 over block light 4.
    fn lit_chunk() -> Chunk {
        let mut chunk = Chunk::empty(0, 0);
        let section = chunk.section_index(64).unwrap();
        Arc::make_mut(&mut chunk.sections[section]).light = Light::packed(Box::new([0xB4; 4096]));
        chunk
    }
    fn lights(view: &super::ProviderLevel, pos: [i32; 3]) -> (u8, u8) {
        (
            view.brightness(LightLayer::Sky, pos),
            view.brightness(LightLayer::Block, pos),
        )
    }
    #[test]
    fn the_providers_light_reads_follow_the_dimensions_sky() {
        for (shape, sky) in [
            (crate::chunk::LevelShape::OVERWORLD, true),
            (crate::chunk::LevelShape::NETHER, false),
        ] {
            let mut scene = Scene::new();
            scene.world.shape = shape;
            scene.world.chunks.insert((0, 0), Arc::new(lit_chunk()));
            let level = scene.level();
            let view = super::ProviderLevel {
                level: &level,
                states: &[],
            };
            let full = if sky { 15 } else { 0 };
            // The stored nibbles, sky 11 and block 4, but a dimension without skylight has no sky layer.
            assert_eq!(lights(&view, [1, 64, 3]), (if sky { 11 } else { 0 }, 4));
            // No chunk is loaded there: the sky the dimension has, and no block light.
            assert_eq!(lights(&view, [100, 64, 3]), (full, 0));
            // Outside the build height the view uses the render world's light read as well.
            // The world owns padding and retained border light (#2761), not the provider.
            assert_eq!(lights(&view, [1, 10_000, 3]), (full, 0));
        }
    }
    /// The provider's view reads a chunk by numeric state ID, using the game's air if absent.
    #[test]
    fn the_providers_level_view_reads_the_loaded_chunks_states() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
            mod_api::__private::resolve_block_keys(["minecraft:air", "minecraft:stone"], |_| {
                sunrise_mod_runtime::block_definitions::BlockDefinition::new(&[&BTreeMap::new()], 0)
                    .map_err(|error| format!("{error:?}"))
            })
            .unwrap();
            let key = |name| mod_api::__private::block_key(name).unwrap();
            let air = key("minecraft:air");
            let stone = key("minecraft:stone");
            let states = [air, stone].map(|block| mod_api::BlockStateRecord::new(block, 0).into());
            let mut scene = Scene::new();
            let mut chunk = Chunk::empty(0, 0);
            chunk.set(1, 64, 3, 1);
            scene.world.chunks.insert((0, 0), Arc::new(chunk));
            let level = scene.level();
            let view = super::ProviderLevel {
                level: &level,
                states: &states,
            };
            assert_eq!(view.block_state([1, 64, 3]).block(), stone);
            assert_eq!(view.block_state([2, 64, 3]).block(), air);
            assert_eq!(view.block_state([100, 64, 3]).block(), air);
        });
    }
    #[test]
    fn a_type_resolves_to_its_provider_at_load_and_other_types_stay_declarative() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-provider-sets-{}", std::process::id()));
        let particles = dir.join("assets/mymod/particles");
        std::fs::create_dir_all(&particles).unwrap();
        for name in ["orbit", "plain"] {
            std::fs::write(
                particles.join(format!("{name}.json")),
                br#"{"textures":["mymod:one","mymod:two"],"sunrise:behaviour":{"lifetime":7,"size":[0.3,0.3]}}"#,
            )
            .unwrap();
        }
        let mut e = engine();
        e.providers
            .insert("mymod:orbit".into(), Arc::new(Probe::default()));
        e.providers
            .insert("mymod:unmatched".into(), Arc::new(Probe::default()));
        e.reload(
            &ResourceSource::directory(dir.clone()),
            &HashMap::new(),
            &HashSet::new(),
        );
        assert!(e.mod_sets["mymod:orbit"].provider.is_some());
        assert!(e.mod_sets["mymod:plain"].provider.is_none());
        assert!(!e.mod_sets.contains_key("mymod:unmatched"));
        let orbit = e
            .make(
                &ParticleOptions::named("mymod:orbit"),
                [0.0; 3],
                [0.0; 3],
                3,
            )
            .unwrap();
        // The declared spawn values seed it, and its sprite is drawn by frame from the set.
        assert!(orbit.provided.is_some());
        assert_eq!((orbit.lifetime, orbit.size), (7, 0.3));
        assert_eq!(orbit.set, Some(e.mod_sets["mymod:orbit"].set));
        let plain = e
            .make(
                &ParticleOptions::named("mymod:plain"),
                [0.0; 3],
                [0.0; 3],
                3,
            )
            .unwrap();
        assert!(plain.provided.is_none());
        assert!(matches!(plain.behaviour, Behaviour::Mod { .. }));
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn controlled_spawn_respects_distance_settings_and_declared_sprite_sets() {
        let mut e = engine();
        assert!(
            e.controlled("example:missing", [0.0; 3], true, Moving(0))
                .is_none()
        );
        assert!(
            e.controlled("example:orbit", [33.0, 0.0, 0.0], true, Moving(0))
                .is_none()
        );
        assert!(
            e.controlled("example:orbit", [f64::NAN, 0.0, 0.0], true, Moving(0))
                .is_none()
        );
        assert!(
            e.controlled("example:orbit", [32.0, 0.0, 0.0], true, Moving(0))
                .is_some()
        );
        e.particle_level = 2;
        assert!(
            e.controlled("example:orbit", [0.0; 3], true, Moving(0))
                .is_none()
        );
    }
}
