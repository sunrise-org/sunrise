//! Shader packs: the one picked on Options > Sunrise Settings > Shaders, or
//! `SUNRISE_SHADER_PACK=<name>` (development; it wins over the pick), runs `shaderpacks/<name>`
//! (a folder or a zip beside `options.txt`) over the world. The programs of the current
//! dimension's folder are loaded when the dimension changes (spec 1.2). A pick in the removed
//! WGSL format is turned off with a note (`shader_pack_unsupported`).

use super::remote_camera::CameraFogEffects;
use sunrise_client::camera::CameraFluid;
use sunrise_client::gfx::pack_uniforms::{PackInputs, PlayerStatus, ViewEntity};
use sunrise_client::shader_options::ShaderOptions;
use sunrise_game_data::text::Text;
use sunrise_items::items::{BlockTagFlags, EntityTypeTags, Items};
use sunrise_mod_package::mod_list::ModList;
use sunrise_shaders::{
    BlockIds, BlockLayers, Macros, Pack, Properties, TagMembers, Tonemap, dimension_folder_named,
    load_post,
};
use sunrise_world::loot::{ItemTagNames, LootTags};

use super::native_shaders::{fingerprint, outcome, refused, remember};
use super::{Dim, Screen};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use sunrise_client_render::graphics::IrisCompat;
use sunrise_client_render::render_geometry::EntitySpan;

/// The values the player chose for pack `name`'s options (`shaderpacks/<name>.txt`).
pub(in crate::game::client_side) fn chosen_shader_options(name: &str) -> BTreeMap<String, String> {
    let file = crate::game::ClientGame::shader_packs_dir().join(sunrise_shaders::chosen_file(name));
    std::fs::read_to_string(file)
        .map(|text| sunrise_shaders::parse_chosen(&text))
        .unwrap_or_default()
}

thread_local! {
    /// The identity scans (`sunrise_shaders::asks_for_identity`) this thread asked for: the main
    /// thread's, for the title screen, the HDR read before the frame, Shader Options and the load.
    static IDENTITY_SCANS: RefCell<IdentityScans> = RefCell::default();
    /// How many scans ran on this thread (`scan_identity`): the tests show the frame runs none.
    #[cfg(test)]
    static SCANS_RUN_HERE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What an identity scan reads: the pack's files (`fingerprint`, `None` when it can't be taken),
/// the options chosen for it and the programs mods request.
#[derive(Clone, PartialEq)]
struct ScanKey {
    files: Option<u64>,
    chosen: BTreeMap<String, String>,
    mod_programs: Vec<String>,
}

/// A pack's answer from its scan: where it asks, if it does.
type Asked = Option<String>;

/// The identity scans by pack path (`IDENTITY_SCANS`): the last one of each pack that landed, with
/// what it read, and the one running on a thread of its own, with what it reads and where its
/// answer comes.
#[derive(Default)]
struct IdentityScans {
    done: HashMap<std::path::PathBuf, (ScanKey, Asked)>,
    running: HashMap<std::path::PathBuf, (ScanKey, std::sync::mpsc::Receiver<Asked>)>,
}

impl IdentityScans {
    /// Whether the scan of the pack at `path` still runs; one that landed is kept (`done`). With
    /// `wait`, it waits for it to land.
    fn running(&mut self, path: &Path, wait: bool) -> bool {
        use std::sync::mpsc::TryRecvError;
        let Some((_, answer)) = self.running.get(path) else {
            return false;
        };
        let asked = match answer.try_recv() {
            Ok(asked) => asked,
            Err(TryRecvError::Empty) if !wait => return true,
            Err(TryRecvError::Empty) => answer.recv().unwrap_or_default(),
            // Its thread ended without an answer (`scan_identity` contains a scan's panic).
            Err(TryRecvError::Disconnected) => None,
        };
        if let Some((key, _)) = self.running.remove(path) {
            self.done.insert(path.to_owned(), (key, asked));
        }
        false
    }

    /// The answer kept for the pack at `path` read as `key` says. One whose fingerprint can't be
    /// taken answers once, the read it was made for: such a pack is scanned at every read.
    fn kept(&mut self, path: &Path, key: &ScanKey) -> Option<Asked> {
        let (kept, asked) = self.done.get(path).filter(|(kept, _)| kept == key)?;
        let asked = asked.clone();
        if kept.files.is_none() {
            self.done.remove(path);
        }
        Some(asked)
    }
}

/// Where pack `name` at `path` asks for the identity macros (Auto), its files, the options chosen
/// for it and the programs mods request as they are now (`ScanKey`): the answer kept for them,
/// else that of a scan started on a thread of its own (`scan_identity`). `None` while that scan
/// runs: the caller goes on without the answer and asks again; with `wait`, it waits for it. A
/// lookup while a scan runs only looks at whether it has landed.
fn auto_identity(path: &Path, name: &str, wait: bool) -> Option<Asked> {
    IDENTITY_SCANS.with_borrow_mut(|scans| {
        loop {
            if scans.running(path, wait) {
                return None;
            }
            let requests = sunrise_client_render::mod_pipelines::pack_program_requests();
            let key = ScanKey {
                files: fingerprint(path),
                chosen: chosen_shader_options(name),
                mod_programs: requests.into_iter().flat_map(|r| r.programs).collect(),
            };
            if let Some(asked) = scans.kept(path, &key) {
                return Some(asked);
            }
            let (send, answer) = std::sync::mpsc::channel();
            let (at, read) = (path.to_owned(), key.clone());
            let spawned = std::thread::Builder::new()
                .name("identity scan".into())
                .spawn(move || _ = send.send(scan_identity(&at, read.chosen, &read.mod_programs)));
            if let Err(e) = spawned {
                log::warn!("shader pack {name}: no thread for its identity scan ({e}); read here");
                return Some(scan_identity(path, key.chosen, &key.mod_programs));
            }
            scans.running.insert(path.to_owned(), (key, answer));
            if !wait {
                return None;
            }
        }
    })
}

/// The identity scan of the pack at `path` with the options `chosen` for it and the programs mods
/// request (`sunrise_shaders::asks_for_identity`), on the calling thread. A pack that can't be
/// opened doesn't ask (its load says why it can't run); a scan that panics doesn't either, and
/// its panic is logged.
fn scan_identity(path: &Path, chosen: BTreeMap<String, String>, mod_programs: &[String]) -> Asked {
    #[cfg(test)]
    SCANS_RUN_HERE.with(|n| n.set(n.get() + 1));
    let pack = Pack::open(path).ok()?.with_chosen(chosen);
    let asked =
        sunrise_level::crash::contain(|| sunrise_shaders::asks_for_identity(&pack, mod_programs));
    asked.unwrap_or_else(|why| {
        log::warn!(
            "shader pack {}: identity scan stopped: {why}",
            path.display()
        );
        None
    })
}

/// The scans still running land; the tests' stand-in for the frames a game draws meanwhile.
#[cfg(test)]
fn await_identity_scans() {
    IDENTITY_SCANS.with_borrow_mut(|scans| {
        let paths: Vec<_> = scans.running.keys().cloned().collect();
        for path in paths {
            scans.running(&path, true);
        }
    });
}

/// The game's frames up to the loader, as `render_with_hooks` runs them: the frame follows the
/// pick, then the loader runs.
#[cfg(test)]
pub(super) fn loader_frame(game: &mut crate::game::ClientGame<'_>) {
    let sky = sunrise_level::environment::Sky::default();
    let camera = glam::DVec3::new(0.5, 65.5, 0.5);
    game.follow_frame();
    game.shader_pack_inputs(camera, camera, 0.0, &sky);
}

/// Frames until the pick runs, installed for `wanted` (its dimension and served answer), with
/// nothing building: a finished build counts only once a frame has taken and installed it.
/// At most 60 s.
#[cfg(test)]
pub(super) fn settle_shader_pack(
    game: &mut crate::game::ClientGame<'_>,
    wanted: (i32, Option<Tonemap>),
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        loader_frame(game);
        let renderer = game.client.renderer.0.as_ref().unwrap();
        if !renderer.shader_pack_building()
            && renderer.shader_pack_kind() == Some("iris")
            && renderer.shader_pack_loaded_for() == Some(wanted)
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the pack never settled"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Why pack `name` at `path` runs with Iris compatibility under the switch `mode` (decision D-8),
/// for the log: On, or Auto and where the pack asks (`auto_identity`); `Some(None)`: it runs
/// without. `None`: on Auto, its scan is still running (never with `wait`).
fn identity(mode: IrisCompat, path: &Path, name: &str, wait: bool) -> Option<Option<String>> {
    match mode {
        IrisCompat::Off => Some(None),
        IrisCompat::On => Some(Some("turned on in Sunrise Settings".into())),
        IrisCompat::Auto => auto_identity(path, name, wait).map(|asked| asked.map(asks)),
    }
}

/// Why a pack that asks runs with Iris compatibility, `at` where it asks.
fn asks(at: String) -> String {
    format!("its programs test them ({at})")
}

/// Whether the pick `name` runs with Iris compatibility under the switch `mode`, its files and
/// options read as they are now (`identity`); `None` while its scan runs (never with `wait`).
fn pick_identity(name: &str, mode: IrisCompat, wait: bool) -> Option<bool> {
    let path = crate::game::ClientGame::shader_packs_dir().join(name);
    Some(identity(mode, &path, name, wait)?.is_some())
}

/// Pack `name`'s options, screens and names in `language`, with the values the player chose,
/// read with `macros` and the identity macros where its Iris compatibility (`mode`) is on, as
/// its load will be: `ShaderOptions::iris_compat` says which.
pub(in crate::game::client_side) fn read_shader_options(
    name: &str,
    language: &str,
    macros: &Macros,
    mode: IrisCompat,
) -> Option<ShaderOptions> {
    let path = crate::game::ClientGame::shader_packs_dir().join(name);
    let pack = match Pack::open(&path) {
        Ok(pack) => pack,
        Err(e) => {
            log::warn!("shader pack {name}: {e}");
            return None;
        }
    };
    let options = sunrise_shaders::options(&pack);
    let lang = sunrise_shaders::Lang::load(&pack, language);
    let chosen = chosen_shader_options(name);
    let pack = pack.with_chosen(chosen.clone());
    // As the load reads it, with the feature macros the pack asks for and its identity answer:
    // the page opened by the player waits for the scan when it is still running.
    let compat = identity(mode, &path, name, true).flatten().is_some();
    let mut macros = macros.clone();
    macros.set_identity(compat);
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let screens = sunrise_shaders::Screens::parse(&properties, &options);
    let mut options = ShaderOptions::new(name.to_owned(), options, screens, lang, chosen);
    options.iris_compat = compat;
    Some(options)
}

/// The macros the Shader Options page reads the pick with, in game and on the title screen:
/// the standard set, then `renderer`'s (`Renderer::define_pack_macros`: its device's feature
/// answers, such as `IRIS_FEATURE_PER_BUFFER_BLENDING`, among them), as the load reads it. With
/// no renderer made yet, the standard set alone, so the device's answers are not there.
pub(in crate::game) fn shader_options_macros(
    renderer: Option<&sunrise_client::gfx::Renderer>,
) -> Macros {
    let mut macros = Macros::standard();
    if let Some(renderer) = renderer {
        renderer.define_pack_macros(&mut macros);
    }
    macros
}

/// Saves the values chosen on the Shader Options page; none chosen removes the file.
/// Writes the chosen options of `options.pack`; whether they were written.
pub(in crate::game::client_side) fn write_shader_options(options: &ShaderOptions) -> bool {
    let file = crate::game::ClientGame::shader_packs_dir()
        .join(sunrise_shaders::chosen_file(&options.pack));
    let saved = if options.chosen.is_empty() {
        std::fs::remove_file(&file).or_else(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(e),
        })
    } else {
        std::fs::write(&file, sunrise_shaders::chosen_text(&options.chosen))
    };
    if let Err(e) = &saved {
        log::warn!("shader pack {}: {}: {e}", options.pack, file.display());
    }
    saved.is_ok()
}

/// The pack that runs over the world ("": none): `SUNRISE_SHADER_PACK`, else the pick
/// (`active_pack`).
pub(in crate::game::client_side) fn dev_pack() -> Option<&'static str> {
    static DEV_PACK: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    DEV_PACK
        .get_or_init(|| std::env::var("SUNRISE_SHADER_PACK").ok())
        .as_deref()
}

pub(in crate::game::client_side) fn picked_pack(
    settings: &sunrise_client::settings::Settings,
) -> &str {
    active_pack(
        dev_pack().unwrap_or(&settings.sunrise.shader_pack),
        &settings.mods,
    )
}

/// What the toggle key decided: the pack it turned off or asked for, a hint when there is
/// nothing (or nothing left) to resume, or nothing when the developer override wins.
/// Announcing "on" is the selection request: the loader still has to accept the pack (its
/// refusal path reports that), and a pack whose owning mod is off never starts.
pub(in crate::game) enum TogglePlan {
    /// Turned `name` off; the chat names it.
    Off(String),
    /// Asked for `name`; the chat names it.
    On(String),
    /// The last pick's owning mod is off: hint with its name, change nothing.
    HintModOff(String),
    /// Nothing to resume: clear `last` (a pack removed after it was turned off) and hint.
    HintClearLast,
    /// Nothing to resume; hint.
    Hint,
    /// The developer override wins; do nothing.
    Noop,
}

/// The toggle key's decision on the pick, without touching the game: `dev_override` is
/// `dev_pack()` (the `SUNRISE_SHADER_PACK` override wins over the pick), `present` says
/// whether the last pick is still installed, and `active` whether its owning mod lets it
/// run (`active_pack`). Pure so tests cover every branch without a game.
pub(in crate::game::client_side) fn plan_shader_toggle(
    pick: &str,
    last: &str,
    dev_override: bool,
    present: impl FnOnce(&str) -> bool,
    active: impl FnOnce(&str) -> bool,
) -> TogglePlan {
    if dev_override {
        return TogglePlan::Noop;
    }
    if !pick.is_empty() {
        return TogglePlan::Off(pick.to_owned());
    }
    if last.is_empty() {
        return TogglePlan::Hint;
    }
    if !present(last) {
        return TogglePlan::HintClearLast;
    }
    if !active(last) {
        return TogglePlan::HintModOff(last.to_owned());
    }
    TogglePlan::On(last.to_owned())
}

/// What the renderer made before any `Game` exists (the loading screen's) readies for the pick,
/// with `pick_hdr` the game then keeps (`NativeShaders::pick_hdr`), so its notes are logged once.
pub(in crate::game) fn startup_pack_shape(
    settings: &sunrise_client::settings::Settings,
    pick_hdr: &mut PickHdr,
) -> PickedPack {
    pack_shape(
        settings,
        super::native_shaders::linear_frame_switch(),
        pick_hdr,
    )
}

/// A pack's id files as the load reads them, each with `macros` as `read_properties` left them
/// (the feature answers among them): `block.properties` (ids and `layer.*` rules, 4.1, 4.2), and
/// `entity.properties` and `item.properties` (`NamedIds::read`).
struct IdFiles {
    blocks: BlockIds,
    layers: BlockLayers,
    named: NamedIds,
    /// The rules of `item.properties` and of `entity.properties`, which `PackIds::resolve`
    /// resolves against the world's tags.
    item_rules: BlockIds,
    entity_rules: BlockIds,
}

fn read_id_files(pack: &Pack, macros: &Macros) -> IdFiles {
    let block_properties = Properties::parse(
        &pack.read_text("block.properties").unwrap_or_default(),
        macros,
    );
    let (named, item_rules, entity_rules) = NamedIds::read(pack, macros);
    IdFiles {
        blocks: BlockIds::parse(&block_properties),
        layers: BlockLayers::parse(&block_properties),
        named,
        item_rules,
        entity_rules,
    }
}

/// The `sunrise.hdr` request of pack `name` at `path` as the frame is picked for it: its
/// `shaders.properties` read with the standard macros, the identity macros where its Iris
/// compatibility (`mode`) is on, and the pack's options as chosen, since the renderer's macros
/// come with the renderer the answer picks; a note for each value the request's keys don't take;
/// and whether the identity answer's scan was still running (`identity` without waiting), so the
/// request was read without it. Nothing when the pack can't be read.
fn read_pick_hdr(
    name: &str,
    path: &Path,
    mode: IrisCompat,
) -> (Option<Tonemap>, Vec<String>, bool) {
    let Ok(pack) = Pack::open(path) else {
        return (None, Vec::new(), false);
    };
    let pack = pack.with_chosen(chosen_shader_options(name));
    // The frame doesn't wait for a scan still running: read without the answer meanwhile.
    let compat = identity(mode, path, name, false);
    let mut macros = Macros::standard();
    macros.set_identity(compat.as_ref().is_some_and(Option::is_some));
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let (hdr, notes) = hdr_request(&properties);
    (hdr, notes, compat.is_none())
}

/// The request in `properties` and its notes, as the log gives them.
fn hdr_request(properties: &Properties) -> (Option<Tonemap>, Vec<String>) {
    let (hdr, notes) = properties.sunrise_hdr();
    let notes = notes
        .into_iter()
        .map(|n| format!("shaders.properties:{}: {}", n.line, n.message))
        .collect();
    (hdr, notes)
}

/// The loader's read of pack `name`'s `shaders.properties` with `macros` (the standard set, the
/// renderer's and the identity answer of its Iris compatibility switch `mode`), and what it logs
/// about it, in order: each required feature Sunrise lacks, each note of the file. Under the
/// switch, also each note about the HDR request not already logged for the pick (`pick_hdr`),
/// and, when the request read here differs from `served`, the answer Sunrise serves the pack
/// (`served_hdr`), that the served answer counts.
/// A load reads the pack's files again: the pick's request is read again at the next frame.
fn loaded_properties(
    pack: &Pack,
    macros: &mut Macros,
    switch: bool,
    served: Option<Tonemap>,
    name: &str,
    mode: IrisCompat,
    pick_hdr: &mut PickHdr,
) -> (Properties, Vec<String>) {
    let (properties, mut log) = sunrise_shaders::read_properties(pack, macros);
    log.extend(
        properties
            .notes
            .iter()
            .map(|n| format!("shaders.properties:{}: {}", n.line, n.message)),
    );
    if switch {
        let (loaded, notes) = hdr_request(&properties);
        pick_hdr.follow(name, mode);
        log.extend(pick_hdr.unreported(notes));
        if loaded != served {
            log.push(format!(
                "shaders.properties asks for Sunrise's HDR as {loaded:?} with the renderer's \
                 macros, but it is served {served:?}: the served answer counts"
            ));
        }
        pick_hdr.forget();
    }
    (properties, log)
}

/// Sunrise's HDR as the loader serves it to the pick: under the switch, the request the
/// renderer's frame was made for (`installed`: `Renderer::pack_hdr_pick`, `Renderer::pack_hdr`)
/// when it was made for this pick (`current`: `picked_pack`, `PickedPack::hdr`) and its request
/// now. A frame kept for another pick, or for this pick's old request, after its rebuild failed
/// (`reconcile_pack_hdr`), serves nothing. The loader's own read of the properties with the
/// renderer's macros doesn't count (`loaded_properties` notes a difference). `None`: not served.
fn served_hdr(
    switch: bool,
    installed: (&str, Option<Tonemap>),
    current: (&str, Option<Tonemap>),
) -> Option<Tonemap> {
    installed.1.filter(|_| switch && installed == current)
}

/// Serves `hdr` (`served_hdr`) to the pack read with `macros` and `inputs`, after its
/// properties were read: its programs see `SUNRISE_HDR` (`Macros::define_sunrise_hdr`), and its
/// `final` is translated as writing linear light (`GameInputs::hdr`). Not served, both are left
/// as they are.
fn serve_hdr(hdr: Option<Tonemap>, macros: &mut Macros, inputs: &mut sunrise_shaders::GameInputs) {
    if let Some(tonemap) = hdr {
        macros.define_sunrise_hdr(tonemap);
    }
    inputs.hdr = hdr;
}

/// The shader pack key (Sunrise Settings' Shaders page), by identity: the translated key's
/// vanilla name against the bound one, so a rebind never becomes another mapping's default.
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn shader_pack_key_press(&mut self, name: &str) -> bool {
        if self.client.screen != Screen::Playing
            || name == "key.keyboard.unknown"
            || self.client.settings.extras.shader_pack_key != name
        {
            return false;
        }
        self.toggle_shader_pack();
        true
    }

    /// What a renderer made now readies for the pick, with the pick's HDR request as last read
    /// (`NativeShaders::pick_hdr`).
    pub(in crate::game) fn picked_pack_shape(&self) -> PickedPack {
        pack_shape(
            &self.client.settings,
            super::native_shaders::linear_frame_switch(),
            &mut self.client.native_shaders.pick_hdr.borrow_mut(),
        )
    }

    /// `RenderAssets::shader_pack_block_id`: the running `shaders/` pack's `block.properties` id
    /// for the block state at registry index `state_idx`, or `None` while no pack runs (the
    /// panorama and a pack still loading included) or the pack gives it none.
    pub(in crate::game) fn active_block_state_id(&self, state_idx: usize) -> Option<i32> {
        running_block_state_id(
            self.shader_pack_active(),
            self.client
                .renderer
                .0
                .as_ref()
                .and_then(|r| r.shader_pack_kind()),
            self.client.pack_ids.as_ref(),
            state_idx,
        )
    }
}

/// What a renderer readies for the pick, unless it was refused, is missing or its owner is off.
#[derive(Clone)]
pub(crate) struct PickedPack {
    /// It draws flat where the device can: the sections are laid out so.
    pub draws_flat: bool,
    /// Its passes read the solid quads' vertices: they start wide.
    pub starts_wide: bool,
    /// Under `SUNRISE_HDR_FRAME=float`, with no pack available or one that asks for Sunrise's
    /// HDR (`hdr`): the frame is the layered float one where HDR output is on (the renderer
    /// decides, `FrameRequest::hdr_float`).
    pub hdr_float: bool,
    /// Under the switch, the running pack's `sunrise.hdr` request (`PickHdr`); the renderer
    /// made for this pick keeps it (`Renderer::pack_hdr`).
    pub hdr: Option<Tonemap>,
    /// The pick (`picked_pack`) this is the shape of: a renderer made for it keeps it with its
    /// HDR request (`Renderer::pack_hdr_pick`).
    pub pick: String,
}
impl PickedPack {
    /// What it asks of the renderer's frame.
    pub(crate) fn frame(&self) -> sunrise_client::gfx::FrameRequest {
        sunrise_client::gfx::FrameRequest::new().with_hdr_float(self.hdr_float)
    }
}

/// The shape of `settings`' pick with `switch` for `SUNRISE_HDR_FRAME=float`, taking its HDR
/// request from `pick_hdr`.
fn pack_shape(
    settings: &sunrise_client::settings::Settings,
    switch: bool,
    pick_hdr: &mut PickHdr,
) -> PickedPack {
    let picked = picked_pack(settings);
    let dir = crate::game::ClientGame::shader_packs_dir();
    let path = dir.join(picked);
    let active = !picked.is_empty()
        && !sunrise_client::shader_pack_list::unsupported(&dir, picked)
        && (!switch
            || (path.exists()
                && outcome(&path, settings.sunrise.iris_compat_of(picked)) != Some(false)))
        && path
            .file_name()
            .is_none_or(|f| !active_pack(&f.to_string_lossy(), &settings.mods).is_empty());
    // Sunrise's HDR is asked for only where it can be had: under the switch, by a pack that runs.
    let hdr = if !switch {
        None
    } else if active {
        pick_hdr.answer(picked, &path, settings.sunrise.iris_compat_of(picked))
    } else {
        // Not running: whatever runs it again reads its files as they are then.
        pick_hdr.forget();
        None
    };
    // A pack draws the terrain flat where the device can, and its passes read the solid quads'
    // vertices: the sections are laid out so.
    PickedPack {
        draws_flat: active,
        starts_wide: active,
        hdr_float: switch && (!active || hdr.is_some()),
        hdr,
        pick: picked.to_owned(),
    }
}

/// The pick's `sunrise.hdr` request under the switch. The frame follows the pick every frame
/// (`follow_frame`), and reading the request reads all the pack's sources for its options. It
/// is read again when the pick changes, after a frame where the pick doesn't run (off, refused,
/// missing, its mod off), and after each point where the pick's files or chosen options can
/// change (`forget`): a load of the pack, an options save, a resource reload, a scan of the
/// packs; and once the identity scan it was read without has landed. Each frame otherwise
/// compares the pick's name (and, while that scan runs, whether it has landed) and reads nothing.
#[derive(Default)]
pub(crate) struct PickHdr {
    /// The pick (`picked_pack`) the rest is for, and its Iris compatibility switch.
    pick: String,
    mode: IrisCompat,
    /// Its request as last read; `None`: to be read.
    hdr: Option<Option<Tonemap>>,
    /// That read was made without the identity answer, whose scan was still running: it is read
    /// again once the scan has landed (`IdentityScans::running`).
    scanning: bool,
    /// The notes about its request already in the log, so a read again doesn't repeat them.
    reported: Vec<String>,
}

impl PickHdr {
    /// The request of pack `picked` at `path`, read unless a read of that pick is kept. A note
    /// about an unknown value is logged once.
    fn answer(&mut self, picked: &str, path: &Path, mode: IrisCompat) -> Option<Tonemap> {
        self.follow(picked, mode);
        if let Some(hdr) = self.hdr {
            let landed =
                self.scanning && !IDENTITY_SCANS.with_borrow_mut(|s| s.running(path, false));
            if !landed {
                return hdr;
            }
        }
        let (hdr, notes, scanning) = read_pick_hdr(picked, path, mode);
        for note in self.unreported(notes) {
            log::warn!("shader pack {picked}: {note}");
        }
        self.hdr = Some(hdr);
        self.scanning = scanning;
        hdr
    }

    /// The request of pack `picked` as last read, without reading anything: `None` when it
    /// wasn't read since the pick changed or `forget`.
    fn kept(&self, picked: &str) -> Option<Tonemap> {
        self.hdr.flatten().filter(|_| self.pick == picked)
    }

    /// The pick's files or chosen options may have changed: the next answer reads them again.
    pub(in crate::game) fn forget(&mut self) {
        self.hdr = None;
    }

    fn follow(&mut self, picked: &str, mode: IrisCompat) {
        if self.pick != picked {
            picked.clone_into(&mut self.pick);
            self.hdr = None;
            self.reported.clear();
        }
        // Another switch reads the request with other macros.
        if self.mode != mode {
            self.mode = mode;
            self.hdr = None;
        }
    }

    /// The notes of `notes` not yet in the log, now counted as in it.
    fn unreported(&mut self, notes: Vec<String>) -> Vec<String> {
        let new: Vec<String> = notes
            .into_iter()
            .filter(|n| !self.reported.contains(n))
            .collect();
        self.reported.extend(new.iter().cloned());
        new
    }
}

/// The pick's HDR request `picked` (`PickedPack::hdr`) where the frame's configuration was
/// looked at again: a frame that `follows` serves it from now on (`installed`,
/// `Renderer::pack_hdr`). Whether the frame is to be made again; until a new one is made, the
/// installed one keeps its own.
pub(in crate::game) fn reconcile_pack_hdr(
    follows: bool,
    installed: (&mut String, &mut Option<Tonemap>),
    picked: (&str, Option<Tonemap>),
) -> bool {
    if follows {
        picked.0.clone_into(installed.0);
        *installed.1 = picked.1;
    }
    !follows
}

// A native mod can bundle a pack as `native-<mod-id>`; its Mods toggle controls it too.
pub(in crate::game::client_side) fn active_pack<'a>(picked: &'a str, mods: &ModList) -> &'a str {
    if picked.strip_prefix("native-").is_some_and(|id| {
        mods.installed
            .iter()
            .any(|m| m.id == id && m.native.is_some() && mods.disabled.contains(id))
    }) {
        ""
    } else {
        picked
    }
}

/// A selected pack may read resource-pack material maps.
pub fn picks_material_pack(settings: &sunrise_client::settings::Settings) -> bool {
    let dev = std::env::var("SUNRISE_SHADER_PACK").ok();
    let picked = active_pack(
        dev.as_deref().unwrap_or(&settings.sunrise.shader_pack),
        &settings.mods,
    );
    if picked.is_empty() {
        return false;
    }
    let dir = crate::game::ClientGame::shader_packs_dir();
    if sunrise_client::shader_pack_list::unsupported(&dir, picked) {
        return false;
    }
    let path = dir.join(picked);
    path.file_name()
        .is_none_or(|f| !active_pack(&f.to_string_lossy(), &settings.mods).is_empty())
}

/// The id table is read only while a `shaders/` (Iris-format) pack is loaded and rendering:
/// `pack_ids` outlives the panorama and is set before the pack's pipelines finish building.
fn running_block_state_id(
    active: bool,
    kind: Option<&str>,
    ids: Option<&PackIds>,
    state_idx: usize,
) -> Option<i32> {
    if !active || kind != Some("iris") {
        return None;
    }
    ids?.block_state_id(state_idx)
}

/// The ids a pack gives entities and block entities (4.1, 5.3), for the spans of the entity
/// geometry.
#[cfg_attr(any(test, feature = "test-support"), derive(Default))]
pub struct PackIds {
    /// `entity.properties` and `item.properties`.
    pub(in crate::game) named: NamedIds,
    pub(in crate::game) blocks: BlockIds,
    /// `item.properties` and `entity.properties` as read, for `resolve`: kept after it only
    /// while they name a tag, the only case a later `resolve` reads them again.
    item_rules: Option<BlockIds>,
    entity_rules: Option<BlockIds>,
    /// The world's tags the ids were resolved against (`resolve`), when a rule names a tag
    /// (`%tag`, 4.1): `sync_pack_ids` resolves them again once the world's tags are replaced.
    tags: Option<WorldTags>,
    /// `blocks`' rules by the block name they reach through the world's tags
    /// (`BlockIds::by_name`), for `blockEntityId`; none while `blocks` names no tag, whose ids
    /// `BlockIds::id` finds by name.
    block_rules: Option<HashMap<String, Vec<usize>>>,
    /// `block.properties`' `layer.*` rules (4.2), as the terrain meshes them.
    pub(in crate::game::client_side) layers: super::pack_layers::PackLayers,
    /// `terrain_state_ids` of the loaded pack: two ids per block state, the model's then the
    /// fluid's (the renderer holds the same table).
    pub(in crate::game) terrain_states: Vec<i32>,
    /// `blockEntityId` by block state, filled as states are drawn.
    pub(in crate::game) block_states: RefCell<HashMap<u32, i32>>,
    pub(in crate::game) missing_block: i32,
    /// `currentRenderedItemId` of each block item (`StateFacts::item` of its block's default
    /// state): that state's `block.properties` id, or 0.
    pub(in crate::game) block_items: HashMap<String, i32>,
    /// `oldHandLight` (3.2): the first held item and light are those of the brighter hand.
    pub(in crate::game) old_hand_light: bool,
    /// `vignette`: `false` hides the game's vignette over the pack's image.
    pub(in crate::game) vignette: bool,
    /// `oldLighting`: `true` keeps the game's face shading in the block colours.
    pub(in crate::game) old_lighting: bool,
    /// `separateAo`: `true` passes AO factor in vertex alpha and keeps pure tint in RGB.
    pub(in crate::game) separate_ao: bool,
}

/// The world's block, item and entity type tables the client reads, as `sync_pack_layers`
/// reads them: the tags its server sent, else the remote's tags, else the game's own.
macro_rules! client_items {
    ($game:expr) => {
        $game
            .common
            .server_items
            .as_deref()
            .or_else(|| $game.client.remote.as_ref()?.tags.as_ref())
            .unwrap_or_else(|| $game.common.res.items())
    };
}

/// The tags a pack's `%tag` entries read (D-9), by tag name.
struct PackTags<'a> {
    blocks: Cow<'a, TagMembers>,
    items: Cow<'a, TagMembers>,
    entities: Cow<'a, TagMembers>,
}

impl PackTags<'_> {
    /// No tags: what a pack whose entries name none reads.
    fn none() -> Self {
        Self {
            blocks: Cow::Owned(TagMembers::new()),
            items: Cow::Owned(TagMembers::new()),
            entities: Cow::Owned(TagMembers::new()),
        }
    }
}

/// Which binding of the world's tags a pack's ids were resolved against. Connected to a server
/// (the window of a single-player world included), the tags it sent: `install_server_items`
/// makes a new `server_items` for each tags packet. Otherwise the world's tables, each from
/// one binding: every path that replaces the block tags rebuilds `block_tag_flags`
/// (`Items::rebuild_block_tag_flags`), the item tags bind `ItemTagNames` again
/// (`Items::set_tags`), and the entity type tags are a new `EntityTypeTags`
/// (`Items::set_entity_type_tags`). Held, so no address is reused.
enum WorldTags {
    Sent(Arc<Items>),
    Tables {
        blocks: Arc<BlockTagFlags>,
        items: Option<ItemTagNames>,
        entities: Arc<EntityTypeTags>,
    },
}

impl WorldTags {
    fn tables(items: &Items) -> Self {
        Self::Tables {
            blocks: items.block_tag_flags.clone(),
            items: items.item_tag_names().cloned(),
            entities: items.entity_type_tags.clone(),
        }
    }

    /// Whether these are still the tags of a world whose server sent `sent` (`None`: not
    /// connected) and whose tables are `items`.
    fn hold(&self, sent: Option<&Arc<Items>>, items: &Items) -> bool {
        match (self, sent) {
            (Self::Sent(held), Some(now)) => Arc::ptr_eq(held, now),
            (
                Self::Tables {
                    blocks,
                    items: names,
                    entities,
                },
                None,
            ) => {
                Arc::ptr_eq(blocks, &items.block_tag_flags)
                    && Arc::ptr_eq(entities, &items.entity_type_tags)
                    && match (names, items.item_tag_names()) {
                        (Some(held), Some(now)) => held.same_generation(now),
                        (None, None) => true,
                        _ => false,
                    }
            }
            _ => false,
        }
    }
}

/// The ids `entity.properties` and `item.properties` give names (4.1), and what a name each
/// file lacks reads (5.3). Each file decides its own default: `entityId` follows
/// `entity.properties` alone, `heldItemId` `item.properties` alone.
#[cfg_attr(any(test, feature = "test-support"), derive(Default))]
pub struct NamedIds {
    /// `entity.properties` (4.1).
    pub(in crate::game) entities: HashMap<String, i32>,
    /// The `entityId` of an entity `entity.properties` lacks: 65535, or 0 without the file.
    pub(in crate::game) missing_entity: i32,
    /// `item.properties` (4.1).
    pub(in crate::game) items: HashMap<String, i32>,
    /// Whether the pack has `item.properties` (`currentRenderedItemId` of an item it lacks).
    pub(in crate::game) item_file: bool,
    /// The id of a held item `item.properties` lacks, or of an empty hand: −1, or 0 without
    /// the file.
    pub(in crate::game) missing_item: i32,
}

impl NamedIds {
    /// Reads the pack's `entity.properties` and `item.properties` with `macros`. Whether the
    /// pack has a file decides its default, even when the file can't be read (a zipped file past
    /// the size cap): such a file gives no ids. The ids are the files' entries by name, with no
    /// tag; with them, the files' rules, items' then entities', which `PackIds::resolve` resolves
    /// against the world's tags (`%tag`, 4.1).
    pub(in crate::game) fn read(pack: &Pack, macros: &Macros) -> (Self, BlockIds, BlockIds) {
        let read = |kind: &str| {
            let path = format!("{kind}.properties");
            let text = pack.read_text(&path);
            // A successful read proves the file is there; only a failed one needs the lookup.
            let present = text.is_some() || pack.exists(&path);
            let rules = BlockIds::parse_kind(
                &Properties::parse(text.as_deref().unwrap_or_default(), macros),
                kind,
            );
            (rules, present)
        };
        let (entity_rules, entity_file) = read("entity");
        let (item_rules, item_file) = read("item");
        let named = Self {
            entities: entity_rules.plain(&TagMembers::new()),
            missing_entity: if entity_file { 65535 } else { 0 },
            items: item_rules.plain(&TagMembers::new()),
            item_file,
            missing_item: if item_file { -1 } else { 0 },
        };
        (named, item_rules, entity_rules)
    }

    /// The `entityId` of entity type `name` (5.3).
    pub(in crate::game) fn entity_id(&self, name: &str) -> i32 {
        self.entities
            .get(name)
            .copied()
            .unwrap_or(self.missing_entity)
    }
}

impl PackIds {
    /// Whether an entry names a tag: only then do the ids follow the world's tags.
    fn names_tags(&self) -> bool {
        let names = |rules: &Option<BlockIds>| rules.as_ref().is_some_and(BlockIds::names_tags);
        self.blocks.names_tags() || names(&self.item_rules) || names(&self.entity_rules)
    }

    /// Resolves the ids against `tags` (a `%tag` entry gives its members the id, 4.1), taken
    /// from the binding `bound`: the terrain's table, the block items', the items' and the
    /// entities', and the block entities' from their next draw on.
    fn resolve(
        &mut self,
        registry: &sunrise_world::registry::BlockRegistry,
        facts: &sunrise_items::blocks::BlockFacts,
        tags: &PackTags<'_>,
        bound: WorldTags,
    ) {
        self.terrain_states = terrain_state_ids(registry, &self.blocks, &tags.blocks);
        self.block_items = block_item_ids(facts, registry, &self.blocks, &tags.blocks);
        self.block_rules = self.blocks.names_tags().then(|| {
            (self.blocks.by_name(&tags.blocks).into_iter())
                .map(|(name, rules)| (name.to_owned(), rules))
                .collect()
        });
        if let Some(rules) = &self.item_rules {
            self.named.items = rules.plain(&tags.items);
        }
        if let Some(rules) = &self.entity_rules {
            self.named.entities = rules.plain(&tags.entities);
        }
        self.block_states.get_mut().clear();
        self.tags = self.names_tags().then_some(bound);
        // Rules naming no tag give the same ids every time: nothing reads them again.
        self.item_rules = self.item_rules.take().filter(BlockIds::names_tags);
        self.entity_rules = self.entity_rules.take().filter(BlockIds::names_tags);
    }

    /// The pack's `block.properties` id for the block state at registry index `state`, the
    /// model id the terrain carries; `None` when the file gives it none or the index is past
    /// the registry.
    pub(in crate::game::client_side) fn block_state_id(&self, state: usize) -> Option<i32> {
        self.terrain_states
            .get(state.checked_mul(2)?)
            .copied()
            .filter(|id| *id >= 0)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl PackIds {
    /// A pack's ids from its `block.properties`, `item.properties` and `entity.properties`
    /// texts, resolved against `game`'s world as the loader resolves them.
    pub fn of_files(
        game: &crate::game::ClientGame<'_>,
        block: &str,
        item: &str,
        entity: &str,
    ) -> Self {
        let parse = |text: &str, kind: &str| {
            BlockIds::parse_kind(&Properties::parse(text, &Macros::standard()), kind)
        };
        let mut ids = Self {
            blocks: parse(block, "block"),
            item_rules: Some(parse(item, "item")),
            entity_rules: Some(parse(entity, "entity")),
            named: NamedIds {
                item_file: true,
                missing_item: -1,
                missing_entity: 65535,
                ..Default::default()
            },
            missing_block: 65535,
            ..Default::default()
        };
        let (tags, bound) = pack_tags(
            game.client.remote.as_ref(),
            game.common.server_items.as_ref(),
            client_items!(game),
            ids.names_tags(),
        );
        ids.resolve(
            game.common.pack.assets.registry,
            game.common.res.facts(),
            &tags,
            bound,
        );
        ids
    }

    pub fn with_entity_ids<'a>(ids: impl IntoIterator<Item = (&'a str, i32)>) -> Self {
        Self {
            named: NamedIds {
                entities: ids
                    .into_iter()
                    .map(|(name, id)| (name.into(), id))
                    .collect(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

/// The tags a pack's `%tag` entries read now (D-9), and which binding they are; none unless
/// `wanted` (a pack whose entries name no tag reads none). Connected to a server (`remote`,
/// whose tags packets made `sent`), the tags it sent; otherwise the world's tables, `items`.
/// Those packets are also `sent`'s tables.
fn pack_tags<'a>(
    remote: Option<&'a crate::game::client_side::remote_client::Remote>,
    sent: Option<&Arc<Items>>,
    items: &'a Items,
    wanted: bool,
) -> (PackTags<'a>, WorldTags) {
    if let (Some(remote), Some(sent)) = (remote, sent) {
        let bound = WorldTags::Sent(sent.clone());
        if !wanted {
            return (PackTags::none(), bound);
        }
        let names = |registry: &str| -> TagMembers {
            let Some(ids) = remote.builtin.get(registry) else {
                return TagMembers::new();
            };
            let tags = remote.registries.tags.get(registry).into_iter().flatten();
            tags.map(|(tag, members)| {
                let members = members.iter().filter_map(|&id| ids.name(id));
                (tag.clone(), members.map(str::to_owned).collect())
            })
            .collect()
        };
        let tags = PackTags {
            blocks: Cow::Owned(names("minecraft:block")),
            items: Cow::Owned(names("minecraft:item")),
            entities: Cow::Owned(names(EntityTypeTags::REGISTRY)),
        };
        return (tags, bound);
    }
    let bound = WorldTags::tables(items);
    if !wanted {
        return (PackTags::none(), bound);
    }
    let tags = PackTags {
        blocks: Cow::Borrowed(&items.block_tags),
        items: Cow::Borrowed(&items.item_tags),
        entities: Cow::Borrowed(&**items.entity_type_tags.members()),
    };
    (tags, bound)
}

impl crate::game::ClientGame<'_> {
    /// Drops the running or building pack when the pick changed to `picked`.
    fn apply_shader_pack_pick(&mut self, picked: &str) {
        if self.client.native_shaders.picked != picked {
            picked.clone_into(&mut self.client.native_shaders.picked);
            self.client.native_shaders.identity_build = None;
            self.client.native_shaders.identity_seen = None;
            self.client.native_shaders.identity_recheck = false;
            self.set_translucent_layers(Vec::new());
            self.client.pack_ids = None;
            if let Some(renderer) = &mut self.client.renderer.0 {
                renderer.drop_shader_packs();
            }
        }
    }
    /// A title screen frame, which loads no pack and resolves no pack path: shaders turned off in
    /// the settings still drop the pack now, and the cancelled builds that have ended are joined,
    /// as `shader_pack_inputs` does in a world, so neither keeps its GPU objects until a world
    /// draws.
    pub(in crate::game::client_side) fn shader_pack_title_frame(&mut self) {
        if picked_pack(&self.client.settings).is_empty() {
            self.apply_shader_pack_pick("");
        }
        if let Some(renderer) = &self.client.renderer.0 {
            renderer.reap_pack_builds();
        }
    }
    /// Resolves the loaded pack's ids again when the world's tags its `%tag` entries (4.1) were
    /// resolved against were replaced (a server's tags, `/reload`, another world), and hands
    /// the renderer the terrain's. Nothing for a pack whose entries name no tag.
    pub fn sync_pack_ids(&mut self) {
        let Some(held) = (self.client.pack_ids.as_ref()).and_then(|ids| ids.tags.as_ref()) else {
            return;
        };
        let sent = (self.client.remote.as_ref()).and(self.common.server_items.as_ref());
        if held.hold(sent, client_items!(self)) {
            return;
        }
        let (tags, bound) = pack_tags(
            self.client.remote.as_ref(),
            self.common.server_items.as_ref(),
            client_items!(self),
            true,
        );
        let Some(ids) = self.client.pack_ids.as_mut() else {
            return;
        };
        let registry = self.common.pack.assets.registry;
        ids.resolve(registry, self.common.res.facts(), &tags, bound);
        drop(tags);
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.set_pack_block_ids(ids.terrain_states.clone());
        }
    }

    /// Loads the pack for the current dimension when it changed, and returns this frame's
    /// inputs to the pack's uniforms.
    pub(in crate::game::client_side) fn shader_pack_inputs(
        &mut self,
        camera: glam::DVec3,
        eye: glam::DVec3,
        partial: f32,
        sky: &sunrise_level::environment::Sky,
    ) -> PackInputs {
        // A `shaders/` pack lights the faces itself unless `oldLighting=true`: sections mesh
        // without the game's face shading (the pack loaded or dropped below shows next frame).
        let pack_lighting = self
            .client
            .pack_ids
            .as_ref()
            .is_some_and(|ids| !ids.old_lighting);
        if self.client.streaming.shader_pack_lighting != pack_lighting {
            self.client.streaming.shader_pack_lighting = pack_lighting;
            self.all_sections_changed();
        }
        // Any `shaders/` pack (its `oldLighting` too) draws fluid tops split as Iris does.
        let shader_pack = self.client.pack_ids.is_some();
        if self.client.streaming.shader_pack != shader_pack {
            self.client.streaming.shader_pack = shader_pack;
            self.all_sections_changed();
        }
        // The renderer keeps the meshes' sources while they carry them, and frees them after.
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.keep_pack_sources(shader_pack);
        }
        let separate_ao = self
            .client
            .pack_ids
            .as_ref()
            .is_some_and(|ids| ids.separate_ao);
        if self.client.streaming.separate_ao != separate_ao {
            self.client.streaming.separate_ao = separate_ao;
            self.all_sections_changed();
        }
        // Its `layer.*` rules (4.2), none without a pack: in the frame after it loads or drops, as
        // the flags above, so one meshing of every section takes them all.
        self.sync_pack_layers();
        // Its ids, before this frame's held items and entities read them.
        self.sync_pack_ids();
        let selected = picked_pack(&self.client.settings).to_owned();
        let dir = Self::shader_packs_dir();
        let path = dir.join(&selected);
        let unsupported = sunrise_client::shader_pack_list::unsupported(&dir, &selected);
        if unsupported {
            self.shader_pack_unsupported(&selected);
        }
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let picked = if selected.is_empty()
            || unsupported
            || active_pack(&file, &self.client.settings.mods).is_empty()
        {
            String::new()
        } else {
            path.to_string_lossy().into_owned()
        };
        self.apply_shader_pack_pick(&picked);
        let dimension = match self.common.dim {
            Dim::Overworld | Dim::Extra(_) => 0,
            Dim::Nether => -1,
            Dim::End => 1,
        };
        // Sunrise's HDR as the renderer serves it to the pick (`served_hdr`), looked up only under
        // the switch and for a pick that runs: the pick's request as this frame's `follow_frame`
        // read it (`PickHdr::kept`), no file read here. A build for another answer is dropped
        // before any result is looked at, so a stale build's refusal never counts against the
        // pick; a build for another dimension is left to be polled, as without the extension.
        let served = match &self.client.renderer.0 {
            Some(renderer)
                if super::native_shaders::linear_frame_switch() && !picked.is_empty() =>
            {
                let request = self.client.native_shaders.pick_hdr.borrow().kept(&selected);
                served_hdr(
                    true,
                    (&renderer.pack_hdr_pick, renderer.pack_hdr),
                    (&selected, request),
                )
            }
            _ => None,
        };
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.drop_superseded_pack_load(served);
        }
        // A pack loaded (or building) with another Iris compatibility switch, or another identity
        // answer, than the pick's now is dropped before it can run (`poll_shader_pack_unless`),
        // and loads again below.
        let mode = self.client.settings.sunrise.iris_compat_of(&selected);
        // The pick's answer read again for the pack list (`refresh_identity`), once its scan has
        // landed.
        if self.client.native_shaders.identity_recheck
            && let Some(seen) = pick_identity(&selected, mode, false)
        {
            self.client.native_shaders.identity_seen = Some(seen);
            self.client.native_shaders.identity_recheck = false;
        }
        let stale = self.client.native_shaders.stale_identity(mode);
        if stale {
            self.client.pack_ids = None;
        }
        // A pack whose pipelines were building runs once they are built, or is turned off.
        if let Some((dimension, name, started, report)) = self
            .client
            .renderer
            .0
            .as_mut()
            .and_then(|r| r.poll_shader_pack_unless(stale))
        {
            match report {
                Ok(Some((programs, notes))) => {
                    remember(&path, fingerprint(&path), mode, true);
                    for note in notes {
                        log::warn!("shader pack {name}: {note}");
                    }
                    log::info!(
                        "shader pack {name}: {} programs ({}) in {} ms",
                        programs.len(),
                        programs.join(" "),
                        started.elapsed().as_millis()
                    );
                }
                Ok(None) => remember(&path, fingerprint(&path), mode, false),
                // Stopped because it isn't wanted any more: not the pack's failure.
                Err(refused) if refused.is_cancelled() => {}
                Err(refused) => {
                    for note in refused.notes {
                        log::warn!("shader pack {name}: {note}");
                    }
                    self.shader_pack_failed(&name, dimension, &refused.reason);
                }
            }
        }
        if picked.is_empty() {
            return PackInputs::default();
        }
        if !sunrise_client::sprites::labpbr_maps() {
            sunrise_client::sprites::set_labpbr_maps(true);
            self.reload_packs();
            return PackInputs::default();
        }
        let name = selected.clone();
        if let Some(renderer) = &mut self.client.renderer.0
            && renderer.shader_pack_loaded_for() != Some((dimension, served))
            && !(super::native_shaders::linear_frame_switch()
                && refused(&path, fingerprint(&path), mode))
            // Its Iris compatibility, decided before anything is translated: on Auto with no
            // answer kept, its scan runs on a thread of its own, and the game draws without the
            // pack until a frame finds the scan landed.
            && let Some(compat) = identity(mode, &path, &name, false)
        {
            let name = name.as_str();
            let started = std::time::Instant::now();
            // A pack is not the game's own code: one that makes the shader compiler (or our
            // translation of it) panic is turned off instead of crashing every world load (#3525).
            let loaded = sunrise_level::crash::contain(|| {
                let chosen = chosen_shader_options(name);
                match Pack::open(&path).map(|p| p.with_chosen(chosen.clone())) {
                    Ok(pack) => {
                        let mut macros = Macros::standard();
                        renderer.define_pack_macros(&mut macros);
                        if let Some(why) = &compat {
                            log::info!(
                                "shader pack {name}: runs with Iris compatibility (IS_IRIS, \
                                 IRIS_VERSION {}): {why}",
                                sunrise_shaders::IRIS_VERSION
                            );
                        }
                        macros.set_identity(compat.is_some());
                        self.client.native_shaders.identity_build = Some((mode, compat.is_some()));
                        self.client.native_shaders.identity_seen = Some(compat.is_some());
                        self.client.native_shaders.identity_recheck = false;
                        // Its conditionals test the pack's options too.
                        let (properties, log) = loaded_properties(
                            &pack,
                            &mut macros,
                            super::native_shaders::linear_frame_switch(),
                            served,
                            name,
                            mode,
                            &mut self.client.native_shaders.pick_hdr.borrow_mut(),
                        );
                        #[cfg(test)]
                        renderer.test_hooks.loaded_log.clone_from(&log);
                        for line in log {
                            log::warn!("shader pack {name}: {line}");
                        }
                        let separate_ao = properties.separate_ao();
                        renderer.set_separate_ao(separate_ao);
                        // Lava's faces draw with the solid terrain's program.
                        renderer.set_pack_lava_layers(
                            ["lava_still", "lava_flow"]
                                .iter()
                                .filter_map(|f| {
                                    let name = format!("minecraft:block/{f}");
                                    self.client
                                        .client_pack
                                        .render
                                        .sprite_layers
                                        .get(&name)
                                        .map(|&l| l as u16)
                                })
                                .collect(),
                        );
                        let folder = dimension_folder_named(&pack, self.common.dim.id(), dimension);
                        // Its terrain programs read the section arena's pages, or else the
                        // normal stream, where they still fit the vertex stage with them.
                        renderer.set_pack_terrain_bindings(
                            sunrise_client::gfx::pack_atlas::TerrainBindings::of(&properties),
                        );
                        let mut inputs = renderer.pack_inputs();
                        serve_hdr(served, &mut macros, &mut inputs);
                        let mut post = load_post(&pack, &folder, &properties, &macros, &inputs);
                        // Textures that name one of the game's: from the resource packs over
                        // the game's own resources.
                        let resources = self.common.pack.source.clone();
                        let read = |namespace: &str, path: &str| {
                            resources.read(&format!("assets/{namespace}/{path}")).ok()
                        };
                        for note in sunrise_shaders::resolve_resources(&mut post.textures, &read) {
                            log::warn!("shader pack {name}: {note}");
                        }
                        let biomes = world_biomes!(self.common);
                        let names: Vec<&str> =
                            biomes.list.iter().map(|b| b.name.as_str()).collect();
                        post.custom.set_biomes(&names);
                        let IdFiles {
                            blocks: ids,
                            layers,
                            named,
                            item_rules: items,
                            entity_rules: entities,
                        } = read_id_files(&pack, &macros);
                        for note in ids.notes.iter().chain(&layers.notes) {
                            log::warn!("shader pack {name}: block.properties: {note}");
                        }
                        let mut pack_ids = PackIds {
                            block_items: HashMap::new(),
                            named,
                            item_rules: Some(items),
                            // OQ-21: on unless the pack turns it off.
                            old_hand_light: properties
                                .get("oldHandLight")
                                .is_none_or(|v| v.trim() != "false"),
                            // On unless the pack turns it off (`vignette=false`).
                            vignette: properties
                                .get("vignette")
                                .is_none_or(|v| v.trim() != "false"),
                            // Off unless the pack turns it on (Iris shades no faces by default).
                            old_lighting: properties
                                .get("oldLighting")
                                .is_some_and(|v| v.trim() == "true"),
                            separate_ao,
                            entity_rules: Some(entities),
                            tags: None,
                            block_rules: None,
                            missing_block: if ids.is_empty() { 0 } else { 65535 },
                            blocks: ids,
                            layers: super::pack_layers::PackLayers::new(name, layers),
                            terrain_states: Vec::new(),
                            block_states: RefCell::default(),
                        };
                        let started_ids = std::time::Instant::now();
                        let (tags, bound) = pack_tags(
                            self.client.remote.as_ref(),
                            self.common.server_items.as_ref(),
                            client_items!(self),
                            pack_ids.names_tags(),
                        );
                        pack_ids.resolve(
                            self.common.pack.assets.registry,
                            self.common.res.facts(),
                            &tags,
                            bound,
                        );
                        drop(tags);
                        let table = pack_ids.terrain_states.clone();
                        self.client.pack_ids = Some(pack_ids);
                        log::info!(
                            "shader pack {name}: mc_Entity for {} of {} block states in {} ms",
                            table.chunks(2).filter(|p| p[0] != -1).count(),
                            table.len() / 2,
                            started_ids.elapsed().as_millis()
                        );
                        log::info!("shader pack {name}: folder `{folder}`, building its pipelines");
                        // The driver compiles the programs on a thread of their own; the game
                        // draws without a pack meanwhile, and the next frames pick it up.
                        renderer.load_shader_pack(dimension, post, table, name.to_owned(), started);
                    }
                    Err(e) => {
                        remember(&path, fingerprint(&path), mode, false);
                        self.client.pack_ids = None;
                        log::warn!("shader pack {name}: {e}");
                        // Without a pack this can't fail.
                        let _ = renderer.set_shader_pack(dimension, None, &[]);
                    }
                }
                Ok(())
            });
            // A pack that can't run is turned off like one that panics.
            if let Err(error) = loaded.flatten() {
                self.shader_pack_failed(name, dimension, &error);
            }
        }
        let lerp = |(old, new): (f32, f32)| old + (new - old) * partial;
        let at = camera.floor().as_ivec3();
        let light = self.common.world.light(at.x, at.y, at.z);
        let biome = self
            .common
            .world
            .chunks
            .get(&(at.x >> 4, at.z >> 4))
            .map(|c| c.biome(at.x, at.y, at.z));
        let (held_items, held_light) = self.held_for_pack();
        let status = self.pack_player_status();
        let (end_flash, end_flash_direction) = self.end_flash_for_pack(partial);
        let info = biome.map(|b| world_biomes!(self).get(b));
        let dimension = self.client.remote.as_ref().map(|r| &r.dimension);
        PackInputs {
            // `Level.getDefaultClockTime`: 0 in a dimension type with no default clock (the
            // Nether's has none), as Iris 1.11.7 reads `worldTime` there (probe pack).
            day_time: if dimension.is_some_and(|d| d.default_clock.is_none()) {
                0
            } else {
                self.common.world.day_time
            },
            rain: lerp(self.common.world.weather.rain_level),
            thunder: lerp(self.common.world.weather.thunder_level),
            eye_in: match self.camera_fluid(camera.to_array()) {
                CameraFluid::None => 0,
                CameraFluid::Water => 1,
                CameraFluid::Lava => 2,
                CameraFluid::PowderSnow => 3,
            },
            blindness: f32::from(u8::from(
                self.common.potions.effects.get("blindness").is_some(),
            )),
            eye_light: [i32::from(light & 15), i32::from(light >> 4)],
            hide_gui: self.client.chat_ui.hide_gui,
            brightness: self.client.settings.gamma as f32,
            biome: info.map_or(-1, |b| {
                let names = world_biomes!(self).list.iter().map(|b| b.name.as_str());
                sunrise_shaders::biome_pack_id(&b.name, names)
            }),
            biome_known: biome.is_some(),
            biome_category: info.map_or(0, |b| sunrise_shaders::biome_category(&b.name)),
            biome_precipitation: info.map_or(0, |b| {
                sunrise_shaders::biome_precipitation(b.precipitation, b.temperature)
            }),
            temperature: info.map_or(0.0, |b| b.temperature),
            rainfall: info.map_or(0.0, |b| b.downfall),
            held_items,
            held_light,
            right_handed: !self.client.settings.main_hand_left,
            on_ground: self
                .common
                .player
                .as_ref()
                .is_some_and(|p| p.ground.on_ground),
            min_y: self.common.world.shape.min_y,
            height: self.common.world.shape.height,
            logical_height: self.common.world.shape.logical_height,
            has_skylight: self.common.world.shape.sky_light,
            has_ceiling: self.common.world.shape.has_ceiling,
            elytra_flying: self.common.player.as_ref().is_some_and(|p| p.fall_flying),
            cloud_height: sunrise_client::gfx::pack_uniforms::cloud_height(sky.cloud_height),
            // Vanilla's drift in float, times its 0.030000001F blocks a tick.
            cloud_time: sunrise_client::clouds::drift(
                self.common.world.game_time,
                f64::from(partial),
                self.client.client_pack.clouds.0,
            ) as f32
                * 0.030_000_001,
            status,
            end_flash,
            end_flash_direction,
            ambient_light: dimension.map_or(0.0, |d| d.ambient_light),
            boss_battle: self.client.boss_overlay.boss_battle(),
            view: self.pack_view_entity(eye, partial),
            still_ticks: self.common.clock.still().then(|| self.visual_ticks()),
            frame_timing: None,
        }
    }

    /// The camera entity's (`Minecraft.getCameraEntity`, [`Self::camera_subject`]) values for
    /// the uniforms tied to the view (#3948), at `eye`, the entity's eye
    /// `Camera.alignWithEntity` placed (the frame's `eyePosition`). The local player's are the
    /// ones the lightmap, the HUD and the cave ambience have; another living entity's are the
    /// effects the server sent for it (its `CameraFogEffects`, the same the fog reads); an
    /// entity that isn't living has none and looks nowhere.
    pub fn pack_view_entity(&self, eye: glam::DVec3, partial: f32) -> ViewEntity {
        let at = eye.floor().as_ivec3();
        let light = self.common.world.light(at.x, at.y, at.z);
        let eye_light = [i32::from(light & 15), i32::from(light >> 4)];
        let other = self.camera_subject().filter(|c| !c.is_local_player);
        let (living, effects, night_vision, mood) = match &other {
            Some(c) => {
                let fx = c.fog_effects;
                let night_vision = fx.night_vision.map_or(0.0, |duration| {
                    super::sculk::night_vision_scale(duration, partial)
                });
                (c.camera.living.is_some(), fx, night_vision, 0.0)
            }
            None => (
                true,
                CameraFogEffects::from_effects(
                    &self.common.potions.effects,
                    self.client.darkness_blend,
                ),
                self.night_vision_intensity(partial),
                self.client.ambience.moodiness,
            ),
        };
        // LivingEntity.getEffectBlendFactor: the fade only while the instance is there.
        let darkness = effects
            .darkness_blend
            .map_or(0.0, |blend| super::sculk::darkness_blend_at(blend, partial));
        ViewEntity {
            living,
            blindness: f32::from(u8::from(effects.blindness.is_some())),
            night_vision,
            darkness: darkness * self.darkness_effect_scale(),
            mood,
            eye_light,
        }
    }

    /// The window's player's state for the player booleans and the status uniforms: the six
    /// Iris also gives programs and the vitals read as the HUD reads them, and the eight only
    /// custom uniforms read (D-2 in `docs/cleanroom/decisions.md`), each as its vanilla method
    /// answers on a client. Asked only while a pack is picked ([`Self::shader_pack_inputs`]
    /// returns before it otherwise).
    pub fn pack_player_status(&mut self) -> PlayerStatus {
        let spectator = self.common.spectator();
        let survival = !self.common.creative && !spectator;
        let invisible = self.own_invisible();
        let riding = self.common.riding.is_some();
        let (glowing, ridden, wet) = self.pack_player_booleans();
        let Some(p) = &mut self.common.player else {
            return PlayerStatus::default();
        };
        PlayerStatus {
            survival,
            health: p.health.health(),
            max_health: p.attribute("minecraft:max_health").unwrap_or(20.0) as f32,
            food: p.health.food.food as f32,
            air: p.air as f32,
            armor: p
                .attribute("minecraft:armor")
                .or_else(|_| p.attribute("armor"))
                .unwrap_or(0.0) as f32,
            spectator,
            riding,
            feet_in_water: p.fluids.was_touching_water,
            swimming: p.pose == sunrise_sim::pose::Pose::Swimming,
            burning: p.is_on_fire() && !spectator,
            hurt: p.health.hurt_animation_time > 0,
            invisible,
            sneaking: p.pose == sunrise_sim::pose::Pose::Crouching,
            sprinting: p.sprinting,
            // `LivingEntity.isAlive`: the window's player is never removed while it is the
            // camera.
            alive: p.health.health() > 0.0,
            glowing,
            in_lava: p.is_in_lava(),
            ridden,
            wet,
        }
    }

    /// `Entity.isCurrentlyGlowing`, `Entity.isVehicle` and `Entity.isInWaterOrRain` of the
    /// window's player, as a client answers them, read without changing anything.
    pub(in crate::game::client_side) fn pack_player_booleans(&self) -> (bool, bool, bool) {
        let id = self.own_entity_id() as u32;
        // `Entity.isCurrentlyGlowing` on a client: shared flag 6 from a server, else the
        // Glowing effect `LivingEntity.isCurrentlyGlowing` sets it from.
        let glowing = self
            .remote_glowing(id)
            .unwrap_or_else(|| self.common.potions.effects.get("glowing").is_some());
        // `Entity.isVehicle`: the server's passenger list, else the passengers here.
        let ridden = self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.has_passengers(id));
        // `Entity.isInWaterOrRain`: `isInWater`, else `isInRain`.
        let wet = self.common.player.as_ref().is_some_and(|p| {
            let bp = sunrise_core::position::BlockPos::new(
                p.position.x.floor() as i32,
                p.position.y.floor() as i32,
                p.position.z.floor() as i32,
            );
            let up = sunrise_core::position::BlockPos::new(
                bp.x,
                p.bounds().max_position().y.floor() as i32,
                bp.z,
            );
            p.fluids.was_touching_water || self.raining_at(bp) || self.raining_at(up)
        });
        (glowing, ridden, wet)
    }

    /// The Shader Options page opened: the picked pack's options, screens and names.
    pub(in crate::game::client_side) fn open_shader_options(&mut self) {
        let macros = shader_options_macros(self.client.renderer.0.as_deref());
        let name = &self.client.settings.sunrise.shader_pack;
        let mode = self.client.settings.sunrise.iris_compat_of(name);
        self.client.settings.shader_options =
            read_shader_options(name, &self.client.settings.language, &macros, mode);
        // What was read is the pick's answer now: a pack replaced under its name that answers
        // otherwise loads again (`NativeShaders::stale_identity`).
        if let Some(o) = &self.client.settings.shader_options
            && o.pack == picked_pack(&self.client.settings)
        {
            self.client.native_shaders.identity_seen = Some(o.iris_compat);
            self.client.native_shaders.identity_recheck = false;
        }
    }

    /// The pick's files may have changed (the pack list read again): its identity answer is read
    /// again, so a pack replaced under its name that answers otherwise loads again. A scan this
    /// starts runs on a thread of its own; the frames after take its answer once it has landed
    /// (`shader_pack_inputs`).
    pub(in crate::game::client_side) fn refresh_identity(&mut self) {
        let name = picked_pack(&self.client.settings);
        if name.is_empty() {
            return;
        }
        let mode = self.client.settings.sunrise.iris_compat_of(name);
        match pick_identity(name, mode, false) {
            Some(seen) => self.client.native_shaders.identity_seen = Some(seen),
            None => self.client.native_shaders.identity_recheck = true,
        }
    }

    /// The Shader Options page closed with changes: they are saved, and the pack loads again.
    pub(in crate::game::client_side) fn save_shader_options(&mut self) {
        let Some(options) = self.client.settings.shader_options.take() else {
            return;
        };
        if write_shader_options(&options) {
            self.client.native_shaders.pick_hdr.get_mut().forget();
        }
        self.client.pack_ids = None;
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.drop_shader_packs();
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_open_shader_options(&mut self) {
        self.open_shader_options();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_save_shader_options(&mut self) {
        self.save_shader_options();
    }

    /// Turns off the pack `name` after loading it panicked or failed: nothing runs in its place,
    /// it is no longer the picked pack (so the next start does not load it again), and the chat
    /// says so.
    pub(in crate::game::client_side) fn shader_pack_failed(
        &mut self,
        name: &str,
        dimension: i32,
        error: &str,
    ) {
        let path = Self::shader_packs_dir().join(name);
        let mode = self.client.settings.sunrise.iris_compat_of(name);
        remember(&path, fingerprint(&path), mode, false);
        log::error!("shader pack {name}: turned off: {error}");
        self.client.pack_ids = None;
        if let Some(renderer) = &mut self.client.renderer.0 {
            // Marks this dimension as loaded, without a pack, so it is not tried again (without
            // a pack this can't fail).
            let _ = renderer.set_shader_pack(dimension, None, &[]);
        }
        if self.client.settings.sunrise.shader_pack == name {
            self.client.settings.sunrise.shader_pack.clear();
            self.client
                .settings
                .save(&sunrise_gameplay_common::game::options_path());
        }
        self.client_system_message(Text::colored(
            format!("Shader pack {name} could not be loaded and was turned off"),
            0xFF5555,
        ));
    }

    /// Turns off the pick `name` in the removed WGSL format: no pack runs in its place, and the
    /// chat says so once. A saved pick falls back to no pack and is saved so; the developer
    /// override (`SUNRISE_SHADER_PACK`) stays as it is.
    fn shader_pack_unsupported(&mut self, name: &str) {
        if dev_pack().is_none() {
            self.client.settings.set_shader_pick("");
            self.client
                .settings
                .save(&sunrise_gameplay_common::game::options_path());
        }
        if self.client.native_shaders.noted != name {
            self.client.native_shaders.noted = name.to_owned();
            log::warn!("shader pack {name}: no longer supported (the WGSL format)");
            self.client_system_message(Text::tr(
                sunrise_client::shader_pack_list::UNSUPPORTED_NOTE,
                &[&name],
            ));
        }
    }

    /// Whether the game's vignette draws: the option, unless the loaded pack turns it off.
    pub(in crate::game::client_side) fn vignette_shown(&self) -> bool {
        self.client.settings.vignette
            && self.client.pack_ids.as_ref().is_none_or(|ids| ids.vignette)
    }

    /// `currentRenderedItemId` of a held `item`, [`rendered_item_id`]'s; 0 without a pack.
    pub fn rendered_item_id(&self, item: &str) -> i32 {
        self.client.pack_ids.as_ref().map_or(0, |ids| {
            rendered_item_id(
                ids.block_items.get(item).copied(),
                ids.named.items.get(item).copied(),
                ids.named.item_file,
            )
        })
    }

    /// `heldItemId(2)` and `heldBlockLightValue(2)`: main hand, then off hand (5.2, 5.3).
    pub(in crate::game::client_side) fn held_for_pack(&mut self) -> ([i32; 2], [i32; 2]) {
        if self.client.pack_ids.is_none() {
            return ([0; 2], [0; 2]);
        }
        let lum = self.luminance();
        let Some(ids) = &self.client.pack_ids else {
            return ([0; 2], [0; 2]);
        };
        let hands = [
            self.common.selected,
            sunrise_gameplay_common::game::held_items::OFFHAND_SLOT,
        ]
        .map(|slot| {
            let item = self.common.stack_in(slot).map(|s| &*s.item);
            let id = item.and_then(|i| ids.named.items.get(i).copied());
            (
                id.unwrap_or(ids.named.missing_item),
                item.map_or(0, |i| i32::from(lum.of(i))),
            )
        });
        let first = if ids.old_hand_light && hands[1].1 > hands[0].1 {
            hands[1]
        } else {
            hands[0]
        };
        ([first.0, hands[1].0], [first.1, hands[1].1])
    }
}

impl crate::game::ClientGame<'_> {
    /// Starts a span of the entity geometry at vertex `start` for entity type `name`
    /// (`minecraft:zombie`; "" for one the pack cannot name). Nothing without a pack.
    pub(in crate::game) fn mark_entity(&self, start: usize, name: &str) {
        self.mark_entity_part(start, name, false);
    }

    /// Starts a span for entity geometry that blends (name tags, shadows).
    pub(in crate::game::client_side) fn mark_translucent_entity(&self, start: usize, name: &str) {
        self.mark_entity_part(start, name, true);
    }

    pub(in crate::game::client_side) fn mark_entity_part(
        &self,
        start: usize,
        name: &str,
        translucent: bool,
    ) {
        if let Some(span) = self.entity_part_span(start, name, translucent) {
            self.push_entity_span(span);
        }
    }

    pub fn entity_part_span(
        &self,
        start: usize,
        name: &str,
        translucent: bool,
    ) -> Option<EntitySpan> {
        let id = match &self.client.pack_ids {
            Some(ids) => ids.named.entity_id(name),
            None if native_spans() => name_id(name),
            None => return None,
        };
        Some(EntitySpan {
            start: start as u32,
            stage: sunrise_shaders::render_stage("ENTITIES"),
            id,
            translucent,
            ..Default::default()
        })
    }

    /// Starts a span for the block entity of block state `state`.
    pub fn mark_block_entity(&self, start: usize, state: u32) {
        let Some(ids) = &self.client.pack_ids else {
            if native_spans() {
                self.push_span(start, "BLOCK_ENTITIES", state as i32);
            }
            return;
        };
        let id = *ids
            .block_states
            .borrow_mut()
            .entry(state)
            .or_insert_with(|| {
                let registry = self.common.pack.assets.registry;
                registry
                    .state(sunrise_world::registry::BlockStateId(state))
                    .and_then(|s| {
                        let properties = |k: &str| s.properties.get(k).map(String::as_str);
                        match &ids.block_rules {
                            Some(by_name) => {
                                let rules = by_name.get(&s.name)?;
                                ids.blocks.pick(rules.iter().copied(), properties)
                            }
                            None => ids.blocks.id(&s.name, properties, |_| false),
                        }
                    })
                    .unwrap_or(ids.missing_block)
            });
        self.push_span(start, "BLOCK_ENTITIES", id);
        let emission = self
            .common
            .res
            .facts()
            .get(sunrise_world::registry::BlockStateId(state))
            .emission;
        if let Some(last) = self.client.entity_spans.borrow_mut().last_mut() {
            last.emissive = emission > 0;
            if let Some(gpu) = self.client.skin_frame.borrow_mut().spans.last_mut() {
                gpu.emissive = emission > 0;
            }
        }
    }

    /// Starts a span for mod model copies (`WorldDrawContext::draw_model_instances`) under a
    /// pack: block entities, as `BlockEntityDrawContext::model` draws the same model, with the
    /// id of a block the pack doesn't list, since no block owns them. Nothing without a pack.
    pub(in crate::game) fn model_copy_span(&self, start: usize) -> Option<EntitySpan> {
        Some(EntitySpan {
            start: start as u32,
            stage: sunrise_shaders::render_stage("BLOCK_ENTITIES"),
            id: self.client.pack_ids.as_ref()?.missing_block,
            ..Default::default()
        })
    }

    /// Starts the span of the local player's model; `hidden` in first person, where only a
    /// pack's shadow pass draws it.
    pub(in crate::game::client_side) fn mark_player(&self, start: usize, hidden: bool) {
        self.mark_entity(start, "minecraft:player");
        if (self.client.pack_ids.is_some() || native_spans())
            && let Some(last) = self.client.entity_spans.borrow_mut().last_mut()
        {
            last.player = true;
            last.hidden = hidden;
            if let Some(gpu) = self.client.skin_frame.borrow_mut().spans.last_mut() {
                gpu.player = true;
                gpu.hidden = hidden;
            }
        }
    }

    /// Marks vertices `start..end` (one entity) as hidden from the camera, for an entity
    /// Sunrise's culling would drop but a pack's shadow pass still draws (`EntitySpan::hide`).
    pub(in crate::game::client_side) fn hide_spans_from(&self, start: usize, end: usize) -> bool {
        EntitySpan::hide(&mut self.client.entity_spans.borrow_mut(), start, end)
    }

    /// Starts a span of geometry that is not an entity (`PARTICLES`, `DESTROY`...).
    pub(in crate::game) fn mark_stage(&self, start: usize, stage: &str) {
        if self.client.pack_ids.is_some() || native_spans() {
            self.push_span(start, stage, 0);
        }
    }

    pub(in crate::game::client_side) fn push_span(&self, start: usize, stage: &str, id: i32) {
        self.push_entity_span(EntitySpan {
            start: start as u32,
            stage: sunrise_shaders::render_stage(stage),
            id,
            ..Default::default()
        });
    }

    fn push_entity_span(&self, span: EntitySpan) {
        let mut spans = self.client.entity_spans.borrow_mut();
        if self.client.pack_ids.is_some() && self.skinning() {
            let mut skin = self.client.skin_frame.borrow_mut();
            let gpu = EntitySpan {
                start: skin.vertices,
                ..span
            };
            match skin.spans.last_mut() {
                Some(last) if last.start == gpu.start => *last = gpu,
                _ => skin.spans.push(gpu),
            }
        }
        // A span that drew nothing gives way to the next.
        match spans.last_mut() {
            Some(last) if last.start == span.start => *last = span,
            _ => spans.push(span),
        }
    }
}

/// `currentRenderedItemId` of an item (Iris 1.11.7, observed with probe packs): a block item
/// (`block`: its block's `block.properties` id, 0 when the file lacks it) reads that, whatever
/// `item.properties` says of it; any other item its `item.properties` id (`item`), and one the
/// file lacks 65535, or 0 when the pack has no `item.properties` (`item_file`).
pub(in crate::game::client_side) fn rendered_item_id(
    block: Option<i32>,
    item: Option<i32>,
    item_file: bool,
) -> i32 {
    block.or(item).unwrap_or(if item_file { 65535 } else { 0 })
}

/// The block items' `currentRenderedItemId`: for each block's default state, the item it gives
/// (`StateFacts::item`) takes the state's id in `ids` (its block's tags in `tags`), or 0. A block
/// named as its item wins over another block giving that item.
pub(in crate::game::client_side) fn block_item_ids(
    facts: &sunrise_items::blocks::BlockFacts,
    registry: &sunrise_world::registry::BlockRegistry,
    ids: &BlockIds,
    tags: &TagMembers,
) -> HashMap<String, i32> {
    let by_name = ids.by_name(tags);
    let mut out = HashMap::new();
    for (i, state) in facts.states.iter().enumerate() {
        let id = sunrise_world::registry::BlockStateId(i as u32);
        if &*state.item == "minecraft:air" || registry.default_state(&state.name) != Some(id) {
            continue;
        }
        let value = registry
            .state(id)
            .and_then(|s| state_id(ids, &by_name, s))
            .unwrap_or(0);
        if state.item == state.name {
            out.insert(state.item.to_string(), value);
        } else {
            out.entry(state.item.to_string()).or_insert(value);
        }
    }
    out
}

/// Spans for a native mod (`native_render::RenderNeeds::entity_spans`), without a pack.
pub(in crate::game::client_side) fn native_spans() -> bool {
    sunrise_client_render::render_slots::needs().entity_spans
}

/// A span's id for entity type `name` without a pack: stable, so a mod can match runs from one
/// frame to the next.
pub(in crate::game::client_side) fn name_id(name: &str) -> i32 {
    // FNV-1a.
    let mut hash: u32 = 0x811c_9dc5;
    for byte in name.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    hash as i32
}

/// The id `ids` give `state`, with `by_name` (`BlockIds::by_name`) the rules reaching each block.
fn state_id(
    ids: &BlockIds,
    by_name: &HashMap<&str, Vec<usize>>,
    state: &sunrise_world::chunk::BlockState,
) -> Option<i32> {
    let rules = by_name.get(state.name.as_str())?;
    ids.pick(rules.iter().copied(), |k| {
        state.properties.get(k).map(String::as_str)
    })
}

/// Model/fluid material ids by the original state, independent of sprite sharing and
/// multipart model selection, a `%tag` rule reaching the blocks `tags` lists under it.
/// Waterlogged model faces retain their model id; emitted fluid faces use the fluid's state
/// (the source state itself for liquid blocks).
pub(in crate::game::client_side) fn terrain_state_ids(
    registry: &sunrise_world::registry::BlockRegistry,
    ids: &BlockIds,
    tags: &TagMembers,
) -> Vec<i32> {
    use sunrise_world::{
        fluid::{FluidKind, FluidState},
        registry::BlockStateId,
    };
    let by_name = ids.by_name(tags);
    let material =
        |state: &sunrise_world::chunk::BlockState| state_id(ids, &by_name, state).unwrap_or(-1);
    (0..registry.len())
        .flat_map(|i| {
            let Some(state) = registry.state(BlockStateId(i as u32)) else {
                return [-1, -1];
            };
            let fluid = FluidState::from_block(state).unwrap_or(FluidState::EMPTY);
            let name = match fluid.kind() {
                FluidKind::Water | FluidKind::FlowingWater => "minecraft:water",
                FluidKind::Lava | FluidKind::FlowingLava => "minecraft:lava",
                FluidKind::Empty => return [material(state), -1],
                FluidKind::Mod(_) | FluidKind::ModFlowing(_) => {
                    return [material(state), material(state)];
                }
            };
            let fluid = if state.name == name {
                Some(state)
            } else {
                registry
                    .default_state(name)
                    .and_then(|id| registry.state(id))
            };
            [material(state), fluid.map_or(-1, material)]
        })
        .collect()
}

#[cfg(test)]
pub(in crate::game) mod terrain_source_tests {
    use crate::game::client_side::shader_packs::*;

    #[test]
    fn terrain_source_cpu_state_filters_fluid_owners_and_pack_reload() {
        let rows = serde_json::json!({"defaults":{"minecraft:air":0,"fixture:signal":1,"minecraft:water":3,"minecraft:lava":5},"states":[
            ["",true,false,"","minecraft:air"],
            ["",false,false,"",{"id":"fixture:signal","properties":{"lit":"false","waterlogged":"false"}}],
            ["",false,true,"",{"id":"fixture:signal","properties":{"lit":"true","waterlogged":"true"}}],
            ["",false,true,"",{"id":"minecraft:water","properties":{"level":"0"}}],
            ["",false,true,"",{"id":"minecraft:water","properties":{"level":"1"}}],
            ["",false,true,"",{"id":"minecraft:lava","properties":{"level":"0"}}]
        ]});
        let registry =
            sunrise_world::registry::BlockRegistry::from_facts_json(rows.to_string().as_bytes())
                .unwrap();
        let parse = |text| BlockIds::parse(&Properties::parse(text, &Macros::standard()));
        let ids = parse(
            "block.31=fixture:signal:lit=false\nblock.47=fixture:signal:lit=true\nblock.59=minecraft:water:level=0\nblock.61=minecraft:water:level=1\nblock.73=minecraft:lava\n",
        );
        assert_eq!(
            terrain_state_ids(&registry, &ids, &TagMembers::new()),
            [-1, -1, 31, -1, 47, 59, 59, 59, 61, 61, 73, 73]
        );
        let changed = parse("block.211=fixture:signal:lit=true\nblock.223=minecraft:water\n");
        assert_eq!(
            terrain_state_ids(&registry, &changed, &TagMembers::new()),
            [-1, -1, -1, -1, 211, 223, 223, 223, 223, 223, -1, -1]
        );
        assert_eq!(
            terrain_state_ids(&registry, &BlockIds::default(), &TagMembers::new()),
            [-1; 12]
        );

        let pack = |ids: &BlockIds| PackIds {
            terrain_states: terrain_state_ids(&registry, ids, &TagMembers::new()),
            ..Default::default()
        };
        let signal = pack(&ids);
        // The model's id, not the fluid's: state 3 is water level 0, state 4 level 1.
        assert_eq!(signal.block_state_id(1), Some(31));
        assert_eq!(signal.block_state_id(2), Some(47));
        assert_eq!(signal.block_state_id(3), Some(59));
        assert_eq!(signal.block_state_id(4), Some(61));
        assert_eq!(signal.block_state_id(5), Some(73));
        assert_eq!(signal.block_state_id(0), None);
        assert_eq!(signal.block_state_id(6), None);
        assert_eq!(signal.block_state_id(usize::MAX), None);
        assert_eq!(pack(&BlockIds::default()).block_state_id(1), None);
        assert_eq!(PackIds::default().block_state_id(1), None);
    }

    /// The fixture of `terrain_source_cpu_state_filters_fluid_owners_and_pack_reload`: air, a
    /// signal unlit and lit (waterlogged), water at levels 0 and 1, lava.
    fn signal_registry() -> sunrise_world::registry::BlockRegistry {
        let rows = serde_json::json!({"defaults":{"minecraft:air":0,"fixture:signal":1,"minecraft:water":3,"minecraft:lava":5},"states":[
            ["",true,false,"","minecraft:air"],
            ["",false,false,"",{"id":"fixture:signal","properties":{"lit":"false","waterlogged":"false"}}],
            ["",false,true,"",{"id":"fixture:signal","properties":{"lit":"true","waterlogged":"true"}}],
            ["",false,true,"",{"id":"minecraft:water","properties":{"level":"0"}}],
            ["",false,true,"",{"id":"minecraft:water","properties":{"level":"1"}}],
            ["",false,true,"",{"id":"minecraft:lava","properties":{"level":"0"}}]
        ]});
        sunrise_world::registry::BlockRegistry::from_facts_json(rows.to_string().as_bytes())
            .unwrap()
    }

    fn tags(rows: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        rows.iter()
            .map(|(tag, members)| {
                let members = members.iter().map(|m| (*m).to_owned()).collect();
                ((*tag).to_owned(), members)
            })
            .collect()
    }

    /// The issue's case on the terrain: `block.<id>=%<tag>` gives the tag's members the id, the
    /// model's and, through a tag naming the fluid, the fluid's; a rule naming the block wins.
    #[test]
    fn terrain_ids_reach_a_tags_members() {
        let registry = signal_registry();
        let ids = BlockIds::parse(&Properties::parse(
            "block.42=%fixture:signals\nblock.43=fixture:signal:lit=true\nblock.59=%water\n",
            &Macros::standard(),
        ));
        let members = |rows: &[(&str, &[&str])]| -> TagMembers {
            tags(rows)
                .into_iter()
                .map(|(tag, m)| (tag, m.into_iter().collect()))
                .collect()
        };
        let world = members(&[
            ("fixture:signals", &["fixture:signal"]),
            ("minecraft:water", &["minecraft:water"]),
        ]);
        assert_eq!(
            terrain_state_ids(&registry, &ids, &world),
            [-1, -1, 42, -1, 43, 59, 59, 59, 59, 59, -1, -1]
        );
        // Without those tags only the named state has an id.
        assert_eq!(
            terrain_state_ids(&registry, &ids, &TagMembers::new()),
            [-1, -1, -1, -1, 43, -1, -1, -1, -1, -1, -1, -1]
        );
    }

    /// `PackIds::resolve` reads the world's block, item and entity type tags, and the tags it
    /// read hold until one of them is bound again; a pack whose entries name no tag keeps none.
    #[test]
    fn pack_ids_follow_the_worlds_tags() {
        use sunrise_items::items::TagRegistry;
        let registry = signal_registry();
        let facts = sunrise_items::blocks::BlockFacts {
            states: Vec::new(),
            air: sunrise_world::registry::BlockStateId(0),
            woods: HashMap::new(),
            sign_texts: HashMap::new(),
        };
        let parse = |text: &str, kind: &str| {
            BlockIds::parse_kind(&Properties::parse(text, &Macros::standard()), kind)
        };
        let mut pack = PackIds {
            blocks: parse("block.42=%fixture:signals\n", "block"),
            item_rules: Some(parse(
                "item.10=%minecraft:swords\nitem.11=minecraft:iron_sword\n",
                "item",
            )),
            entity_rules: Some(parse("entity.20=%skeletons\n", "entity")),
            ..Default::default()
        };
        let mut items = Items::empty();
        items.set_tags(
            TagRegistry::Block,
            tags(&[("fixture:signals", &["fixture:signal"])]),
        );
        items.set_tags(
            TagRegistry::Item,
            tags(&[(
                "minecraft:swords",
                &["minecraft:iron_sword", "minecraft:golden_sword"],
            )]),
        );
        items.set_tags(
            TagRegistry::EntityType,
            tags(&[("minecraft:skeletons", &["minecraft:skeleton"])]),
        );
        pack.block_states.get_mut().insert(1, 99);
        let resolve = |pack: &mut PackIds, items: &Items| {
            let (tags, bound) = pack_tags(None, None, items, pack.names_tags());
            pack.resolve(&registry, &facts, &tags, bound);
        };
        resolve(&mut pack, &items);
        assert_eq!(
            pack.terrain_states,
            [-1, -1, 42, -1, 42, -1, -1, -1, -1, -1, -1, -1]
        );
        assert_eq!(
            pack.named.items,
            HashMap::from([
                ("minecraft:iron_sword".to_owned(), 11),
                ("minecraft:golden_sword".to_owned(), 10),
            ])
        );
        assert_eq!(
            pack.named.entities,
            HashMap::from([("minecraft:skeleton".to_owned(), 20)])
        );
        assert!(
            pack.block_states.get_mut().is_empty(),
            "drawn ids forgotten"
        );
        let held = pack.tags.as_ref().expect("its entries name tags");
        assert!(held.hold(None, &items));
        assert!(
            held.hold(None, &items.clone()),
            "a copy holds the same tags"
        );

        // Each kind bound again, even to the same members, is other tags.
        let mut blocks = items.clone();
        blocks.set_tags(
            TagRegistry::Block,
            tags(&[("fixture:signals", &["fixture:signal"])]),
        );
        let mut swords = items.clone();
        swords.set_tags(
            TagRegistry::Item,
            tags(&[("minecraft:swords", &["minecraft:iron_sword"])]),
        );
        let mut skeletons = items.clone();
        skeletons.set_tags(
            TagRegistry::EntityType,
            tags(&[("minecraft:skeletons", &["minecraft:stray"])]),
        );
        for (name, other) in [
            ("block", &blocks),
            ("item", &swords),
            ("entity", &skeletons),
        ] {
            assert!(!held.hold(None, other), "{name} tags bound again");
        }

        // Resolved against the new tags: the stray is a skeleton now, the golden sword no
        // sword, and the signals keep their id.
        let mut changed = skeletons.clone();
        changed.set_tags(
            TagRegistry::Item,
            tags(&[("minecraft:swords", &["minecraft:iron_sword"])]),
        );
        resolve(&mut pack, &changed);
        assert_eq!(
            pack.named.items,
            HashMap::from([("minecraft:iron_sword".to_owned(), 11)])
        );
        assert_eq!(
            pack.named.entities,
            HashMap::from([("minecraft:stray".to_owned(), 20)])
        );
        assert!(pack.tags.as_ref().unwrap().hold(None, &changed));

        // Entries naming no tag: nothing to follow.
        let mut named = PackIds {
            item_rules: Some(parse("item.11=minecraft:iron_sword\n", "item")),
            ..Default::default()
        };
        resolve(&mut named, &items);
        assert!(named.tags.is_none());
        assert_eq!(
            named.named.items,
            HashMap::from([("minecraft:iron_sword".to_owned(), 11)])
        );
        // Nothing is kept for a later resolve: the rules are dropped, and no index is built.
        assert!(named.item_rules.is_none() && named.entity_rules.is_none());
        assert!(named.block_rules.is_none());

        // A pack whose item entries alone name a tag keeps those, and drops its entity
        // entries, which give the same ids every time.
        let mut items_only = PackIds {
            blocks: parse("block.42=fixture:signal\n", "block"),
            item_rules: Some(parse("item.10=%minecraft:swords\n", "item")),
            entity_rules: Some(parse("entity.21=minecraft:stray\n", "entity")),
            ..Default::default()
        };
        resolve(&mut items_only, &items);
        assert!(items_only.tags.is_some());
        assert!(items_only.item_rules.is_some() && items_only.entity_rules.is_none());
        assert!(
            items_only.block_rules.is_none(),
            "its block entries name no tag"
        );
        assert_eq!(
            items_only.named.entities,
            HashMap::from([("minecraft:stray".to_owned(), 21)])
        );
        resolve(&mut items_only, &changed);
        assert_eq!(
            items_only.named.entities,
            HashMap::from([("minecraft:stray".to_owned(), 21)]),
            "the dropped entries' ids stay"
        );
        assert_eq!(
            items_only.named.items,
            HashMap::from([("minecraft:iron_sword".to_owned(), 10)])
        );
    }

    #[test]
    fn running_block_state_id_needs_a_running_iris_pack() {
        let ids = PackIds {
            terrain_states: vec![-1, -1, 31, -1],
            ..Default::default()
        };
        assert_eq!(
            running_block_state_id(true, Some("iris"), Some(&ids), 1),
            Some(31)
        );
        // Loaded but suppressed (title screen) or not yet rendering.
        assert_eq!(
            running_block_state_id(false, Some("iris"), Some(&ids), 1),
            None
        );
        assert_eq!(running_block_state_id(true, None, Some(&ids), 1), None);
        assert_eq!(
            running_block_state_id(true, Some("native"), Some(&ids), 1),
            None
        );
        assert_eq!(running_block_state_id(true, Some("iris"), None, 1), None);
    }
}

#[cfg(test)]
pub(in crate::game) mod rendered_item_tests {
    use super::rendered_item_id;

    /// Iris 1.11.7's values (probe packs of ours): a block item reads its block's id, and 0 when
    /// `block.properties` lacks it even if `item.properties` names it (oak planks as item 15);
    /// other items read their `item.properties` id, or 65535 when the file lacks them, and 0
    /// when the pack has no such file.
    #[test]
    fn a_held_item_reads_its_block_id_or_item_id() {
        // Stone: block 21 and item 12.
        assert_eq!(rendered_item_id(Some(21), Some(12), true), 21);
        assert_eq!(rendered_item_id(Some(0), Some(15), true), 0);
        assert_eq!(rendered_item_id(Some(0), None, true), 0);
        assert_eq!(rendered_item_id(Some(0), None, false), 0);
        assert_eq!(rendered_item_id(None, Some(11), true), 11);
        assert_eq!(rendered_item_id(None, None, true), 65535);
        assert_eq!(rendered_item_id(None, None, false), 0);
    }
}

#[cfg(test)]
pub(in crate::game) mod native_pack_tests {
    use super::{
        PickHdr, active_pack, loaded_properties, pack_shape, read_id_files, reconcile_pack_hdr,
        startup_pack_shape,
    };
    use sunrise_client_render::graphics::IrisCompat;
    use sunrise_mod_package::mod_list::{Installed, ModList, NativePage};
    use sunrise_shaders::Tonemap;

    #[test]
    fn unknown_pack_is_optimistic_but_refusal_owner_and_presence_override_it() {
        use crate::game::native_shaders::{fingerprint, linear_frame_switch, remember};
        use sunrise_client::settings::Settings;
        let mut settings = Settings::default();
        settings.sunrise.hdr = sunrise_client_render::graphics::HdrMode::On;
        settings.mods.installed.push(Installed {
            id: "frame_owner".into(),
            title: "Frame Owner".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(NativePage::default()),
        });
        let root =
            std::env::temp_dir().join(format!("sunrise-frame-policy-{}", std::process::id()));
        let path = root.join("native-frame_owner");
        std::fs::create_dir_all(path.join("shaders")).unwrap();
        settings.sunrise.shader_pack = path.to_string_lossy().into_owned();
        let switch = linear_frame_switch();
        // A pack nobody answered for draws; it asks no float frame.
        let shape = startup_pack_shape(&settings, &mut PickHdr::default());
        assert!(shape.draws_flat && shape.starts_wide);
        assert!(!shape.hdr_float);
        remember(&path, fingerprint(&path), IrisCompat::Auto, true);
        assert!(!startup_pack_shape(&settings, &mut PickHdr::default()).hdr_float);
        // Refused, the frame is the one no pack asks for.
        remember(&path, fingerprint(&path), IrisCompat::Auto, false);
        let shape = startup_pack_shape(&settings, &mut PickHdr::default());
        assert_eq!(shape.draws_flat, !switch);
        assert_eq!(shape.hdr_float, switch);
        remember(&path, fingerprint(&path), IrisCompat::Auto, true);
        // Its mod off, it does not run.
        settings.mods.disabled.insert("frame_owner".into());
        let shape = startup_pack_shape(&settings, &mut PickHdr::default());
        assert!(!shape.draws_flat && !shape.starts_wide);
        assert_eq!(shape.hdr_float, switch);
        settings.mods.disabled.clear();
        assert!(startup_pack_shape(&settings, &mut PickHdr::default()).draws_flat);
        // Gone, it does not run under the switch.
        std::fs::remove_dir_all(root).unwrap();
        let missing = startup_pack_shape(&settings, &mut PickHdr::default());
        assert_eq!(missing.draws_flat, !switch);
        assert_eq!(missing.hdr_float, switch);
    }

    /// A pack folder under a fresh `root` with `properties` as its `shaders.properties`.
    fn hdr_pack(root: &std::path::Path, name: &str, properties: &str) -> std::path::PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(path.join("shaders")).unwrap();
        std::fs::write(path.join("shaders/shaders.properties"), properties).unwrap();
        path
    }

    fn hdr_settings() -> sunrise_client::settings::Settings {
        let mut settings = sunrise_client::settings::Settings::default();
        settings.sunrise.hdr = sunrise_client_render::graphics::HdrMode::On;
        settings
    }

    /// `pack_shape` for the pick `path`, with `state` as the game's kept request.
    fn shape_of(
        settings: &mut sunrise_client::settings::Settings,
        state: &mut PickHdr,
        path: &std::path::Path,
        switch: bool,
    ) -> (bool, Option<Tonemap>) {
        settings.sunrise.shader_pack = path.to_string_lossy().into_owned();
        let shape = pack_shape(settings, switch, state);
        assert!(shape.draws_flat, "{path:?} runs");
        (shape.hdr_float, shape.hdr)
    }

    /// Under `SUNRISE_HDR_FRAME=float`, a running pack whose `shaders.properties` asks for
    /// Sunrise's HDR (with its options as chosen) gets the float frame; one that doesn't gets
    /// today's pack frame, and without the switch nothing is read.
    #[test]
    fn a_pack_asking_for_hdr_gets_the_float_frame_under_the_switch() {
        let root = std::env::temp_dir().join(format!("sunrise-hdr-request-{}", std::process::id()));
        let plain = hdr_pack(&root, "plain", "separateAo=true\n");
        let asks = hdr_pack(&root, "asks", "sunrise.hdr = true\n");
        let curve = hdr_pack(
            &root,
            "curve",
            "sunrise.hdr = true\nsunrise.hdr.tonemap = sunrise\n",
        );
        let unknown = hdr_pack(&root, "unknown", "sunrise.hdr = on\n");
        let mut settings = hdr_settings();
        let mut state = PickHdr::default();
        let mut shape = |path: &std::path::Path, switch: bool| {
            shape_of(&mut settings, &mut state, path, switch)
        };
        assert_eq!(shape(&asks, false), (false, None));
        assert_eq!(shape(&plain, true), (false, None));
        assert_eq!(shape(&unknown, true), (false, None));
        assert_eq!(shape(&curve, true), (true, Some(Tonemap::Sunrise)));
        assert_eq!(shape(&asks, true), (true, Some(Tonemap::Pack)));
        // Without the switch nothing is read: the kept read is still `asks`'.
        assert_eq!(shape(&plain, false), (false, None));
        std::fs::write(asks.join("shaders/shaders.properties"), "x=1\n").unwrap();
        assert_eq!(shape(&asks, true), (true, Some(Tonemap::Pack)));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The pick's HDR read before the frame is made and the Shader Options page read
    /// `shaders.properties` as the load does: what a flag the pack lists guards (spec 3.2,
    /// decision D-7) is read; what an unlisted flag guards is not.
    #[test]
    fn the_hdr_read_and_the_options_page_see_the_features_the_pack_lists() {
        let root = std::env::temp_dir().join(format!("sunrise-hdr-feature-{}", std::process::id()));
        let guarded = "#ifdef IRIS_FEATURE_SSBO\nsunrise.hdr = true\nsliders = LEVEL\n#endif\n";
        let lists = hdr_pack(
            &root,
            "lists",
            &format!("iris.features.optional = SSBO\n{guarded}"),
        );
        let unlisted = hdr_pack(&root, "unlisted", guarded);
        for path in [&lists, &unlisted] {
            std::fs::write(
                path.join("shaders/options.glsl"),
                "#define LEVEL 2 // [1 2]\n",
            )
            .unwrap();
        }
        let mut settings = hdr_settings();
        let mut state = PickHdr::default();
        assert_eq!(
            shape_of(&mut settings, &mut state, &lists, true),
            (true, Some(Tonemap::Pack))
        );
        assert_eq!(
            shape_of(&mut settings, &mut state, &unlisted, true),
            (false, None)
        );
        let sliders = |path: &std::path::Path| {
            let name = path.to_string_lossy();
            let options = super::read_shader_options(
                &name,
                "en_us",
                &sunrise_shaders::Macros::standard(),
                IrisCompat::Auto,
            );
            options
                .unwrap()
                .screens
                .sliders
                .into_iter()
                .collect::<Vec<_>>()
        };
        assert_eq!(sliders(&lists), ["LEVEL"]);
        assert!(sliders(&unlisted).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Iris compatibility (decision D-8) as the title screen's and the game's Shader Options page
    /// and the HDR read before the frame see it: on Auto a pack whose program tests `IS_IRIS`
    /// gets the identity macros, in its `shaders.properties` too, and one that doesn't gets none;
    /// On and Off decide for the pack they are chosen for, by its name, and Off holds after the
    /// pack is replaced under that name. A check only a saved option reaches asks once it is
    /// saved, and one the pack's files drop no longer does.
    #[test]
    fn iris_compatibility_follows_the_switch_for_the_options_page_and_the_hdr_read() {
        let root = std::env::temp_dir().join(format!("sunrise-iris-compat-{}", std::process::id()));
        let guarded = "#ifdef IS_IRIS\n#if IRIS_VERSION == 11103\nsunrise.hdr = true\n\
                       sliders = LEVEL\n#endif\n#endif\n";
        let asks = hdr_pack(&root, "asks", guarded);
        let plain = hdr_pack(&root, "plain", guarded);
        let program = |path: &std::path::Path, fsh: &str| {
            std::fs::write(
                path.join("shaders/options.glsl"),
                "#define LEVEL 2 // [1 2]\n",
            )
            .unwrap();
            std::fs::write(path.join("shaders/final.fsh"), fsh).unwrap();
            std::fs::write(path.join("shaders/final.vsh"), "void main() {}\n").unwrap();
        };
        program(
            &asks,
            "#include \"/options.glsl\"\n#ifndef IS_IRIS\n#endif\n",
        );
        program(&plain, "#include \"/options.glsl\"\nvoid main() {}\n");
        let read = |settings: &sunrise_client::settings::Settings, path: &std::path::Path| {
            let name = path.to_string_lossy();
            let mode = settings.sunrise.iris_compat_of(&name);
            let options = super::read_shader_options(
                &name,
                "en_us",
                &sunrise_shaders::Macros::standard(),
                mode,
            )
            .unwrap();
            let sliders: Vec<String> = options.screens.sliders.iter().cloned().collect();
            let mut settings = settings.clone();
            let hdr = shape_of(&mut settings, &mut PickHdr::default(), path, true).1;
            (options.iris_compat, sliders, hdr)
        };
        let on = (true, vec!["LEVEL".to_owned()], Some(Tonemap::Pack));
        let off = (false, Vec::new(), None);
        let mut settings = hdr_settings();
        assert_eq!(read(&settings, &asks), on, "Auto, asks");
        assert_eq!(read(&settings, &plain), off, "Auto, doesn't ask");
        let name = |path: &std::path::Path| path.to_string_lossy().into_owned();
        settings
            .sunrise
            .set_iris_compat(&name(&asks), IrisCompat::Off);
        settings
            .sunrise
            .set_iris_compat(&name(&plain), IrisCompat::On);
        assert_eq!(read(&settings, &asks), off, "Off");
        assert_eq!(read(&settings, &plain), on, "On");
        // The game's kept HDR request is read again when the switch changes.
        let mut kept = PickHdr::default();
        settings
            .sunrise
            .set_iris_compat(&name(&asks), IrisCompat::Auto);
        let hdr = |settings: &sunrise_client::settings::Settings, kept: &mut PickHdr| {
            shape_of(&mut settings.clone(), kept, &asks, true).1
        };
        assert_eq!(hdr(&settings, &mut kept), Some(Tonemap::Pack));
        settings
            .sunrise
            .set_iris_compat(&name(&asks), IrisCompat::Off);
        assert_eq!(hdr(&settings, &mut kept), None, "kept, switched off");
        // Replaced under its name, the pack keeps Off.
        std::fs::remove_dir_all(&asks).unwrap();
        let asks = hdr_pack(&root, "asks", guarded);
        program(
            &asks,
            "#include \"/options.glsl\"\n#if IRIS_VERSION > 1\n#endif\n",
        );
        assert_eq!(read(&settings, &asks), off, "Off, replaced");
        settings
            .sunrise
            .set_iris_compat(&name(&asks), IrisCompat::Auto);
        assert_eq!(read(&settings, &asks), on, "Auto again");
        // A check behind an option asks once the option is saved on.
        let behind = hdr_pack(&root, "behind", guarded);
        program(
            &behind,
            "//#define COLORED\n#ifdef COLORED\n#ifdef IS_IRIS\n#endif\n#endif\n",
        );
        assert_eq!(read(&settings, &behind), off, "option off");
        let chosen = behind.with_extension("txt");
        std::fs::write(&chosen, "COLORED=true\n").unwrap();
        assert_eq!(read(&settings, &behind), on, "option saved on");
        // The pack's files drop the check: it no longer asks.
        program(
            &behind,
            "//#define COLORED\n#ifdef COLORED\nfloat x;\n#endif\n// files changed\n",
        );
        assert_eq!(read(&settings, &behind), off, "files changed");
        std::fs::remove_file(chosen).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The frame never runs the identity scan (D-8) nor waits for it. With nothing kept, the HDR
    /// read before the frame (`pack_shape`) starts the scan on a thread of its own and reads the
    /// request without the identity macros meanwhile, and so does the load's answer (`identity`
    /// without waiting), which is `None` until the scan lands; once it has, the next frame's
    /// read takes its answer, and so does the load. A scan kept answers at once, and none runs
    /// on this thread throughout.
    #[test]
    fn the_frame_never_runs_a_cold_identity_scan() {
        let root = std::env::temp_dir().join(format!("sunrise-iris-frame-{}", std::process::id()));
        let guarded = "#ifdef IS_IRIS\nsunrise.hdr = true\n#endif\n";
        let [hdr, load] = ["hdr", "load"].map(|name| {
            let path = hdr_pack(&root, name, guarded);
            std::fs::write(path.join("shaders/final.fsh"), "#ifdef IS_IRIS\n#endif\n").unwrap();
            std::fs::write(path.join("shaders/final.vsh"), "void main() {}\n").unwrap();
            path
        });
        let here = || super::SCANS_RUN_HERE.with(std::cell::Cell::get);
        let mut settings = hdr_settings();
        let mut kept = PickHdr::default();
        assert_eq!(
            shape_of(&mut settings, &mut kept, &hdr, true).1,
            None,
            "scan running"
        );
        let name = load.to_string_lossy().into_owned();
        assert_eq!(super::identity(IrisCompat::Auto, &load, &name, false), None);
        super::await_identity_scans();
        assert_eq!(
            shape_of(&mut settings, &mut kept, &hdr, true).1,
            Some(Tonemap::Pack),
            "read again once the scan landed"
        );
        let answer = super::identity(IrisCompat::Auto, &load, &name, false);
        assert_eq!(
            answer,
            Some(Some("its programs test them (final.fsh:1)".into()))
        );
        let mut fresh = PickHdr::default();
        assert_eq!(
            shape_of(&mut settings, &mut fresh, &hdr, true).1,
            Some(Tonemap::Pack),
            "kept"
        );
        assert_eq!(here(), 0, "scans run on this thread");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A pack loaded with one Iris compatibility switch and answer is stale once the pick's
    /// switch is another, or once the pick's answer read again differs (the pack replaced under
    /// its name), so the game drops it before its poll can install it; once dropped, or with the
    /// same switch and answer, it isn't. Nothing loaded is never stale.
    #[test]
    fn a_load_from_other_inputs_than_the_picks_is_dropped() {
        let mut shaders = crate::game::native_shaders::NativeShaders::default();
        assert!(!shaders.stale_identity(IrisCompat::Off));
        shaders.identity_build = Some((IrisCompat::Auto, true));
        shaders.identity_seen = Some(true);
        assert!(!shaders.stale_identity(IrisCompat::Auto));
        assert!(shaders.stale_identity(IrisCompat::Off));
        assert_eq!(shaders.identity_build, None);
        assert!(!shaders.stale_identity(IrisCompat::Off));
        shaders.identity_build = Some((IrisCompat::On, true));
        assert!(shaders.stale_identity(IrisCompat::Auto));
        shaders.identity_build = Some((IrisCompat::Auto, true));
        shaders.identity_seen = Some(false);
        assert!(
            shaders.stale_identity(IrisCompat::Auto),
            "answered otherwise"
        );
    }

    /// A pack replaced under its name whose new files no longer ask, with the same HDR request:
    /// the pick's answer read again (`pick_identity`, as the pack list and Shader Options read
    /// it) differs from the one its build ran with, so the build is stale and loads again.
    #[test]
    fn a_pack_replaced_under_its_name_with_another_answer_loads_again() {
        let root =
            std::env::temp_dir().join(format!("sunrise-iris-replaced-{}", std::process::id()));
        let path = hdr_pack(&root, "replaced", "sunrise.hdr = true\n");
        std::fs::write(path.join("shaders/final.fsh"), "#ifdef IS_IRIS\n#endif\n").unwrap();
        std::fs::write(path.join("shaders/final.vsh"), "void main() {}\n").unwrap();
        let name = path.to_string_lossy().into_owned();
        let mut shaders = crate::game::native_shaders::NativeShaders::default();
        let seen = super::pick_identity(&name, IrisCompat::Auto, true);
        assert_eq!(seen, Some(true));
        shaders.identity_build = Some((IrisCompat::Auto, true));
        shaders.identity_seen = seen;
        assert!(!shaders.stale_identity(IrisCompat::Auto));
        // Replaced: the same name and HDR request, a program that no longer asks.
        std::fs::remove_dir_all(&path).unwrap();
        let path = hdr_pack(&root, "replaced", "sunrise.hdr = true\n");
        std::fs::write(path.join("shaders/final.fsh"), "void main() {}\n").unwrap();
        std::fs::write(path.join("shaders/final.vsh"), "void main() {}\n").unwrap();
        let (mut a, mut b) = (super::PickHdr::default(), super::PickHdr::default());
        assert_eq!(
            a.answer(&name, &path, IrisCompat::Auto),
            Some(Tonemap::Pack)
        );
        shaders.identity_seen = super::pick_identity(&name, IrisCompat::Auto, true);
        assert_eq!(shaders.identity_seen, Some(false));
        assert!(shaders.stale_identity(IrisCompat::Auto));
        assert_eq!(
            b.answer(&name, &path, IrisCompat::Auto),
            Some(Tonemap::Pack)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A pack whose fingerprint can't be taken (a dangling link among its files) is scanned
    /// again at every read: an edit that drops its check is seen, though the fingerprint fails
    /// the same way before and after.
    #[cfg(unix)]
    #[test]
    fn a_pack_without_a_fingerprint_is_scanned_every_time() {
        let root =
            std::env::temp_dir().join(format!("sunrise-iris-dangling-{}", std::process::id()));
        let path = hdr_pack(&root, "dangling", "");
        std::os::unix::fs::symlink(root.join("gone"), path.join("shaders/gone.glsl")).unwrap();
        assert_eq!(crate::game::native_shaders::fingerprint(&path), None);
        let name = path.to_string_lossy().into_owned();
        std::fs::write(path.join("shaders/final.fsh"), "#ifdef IS_IRIS\n#endif\n").unwrap();
        std::fs::write(path.join("shaders/final.vsh"), "void main() {}\n").unwrap();
        assert_eq!(
            super::pick_identity(&name, IrisCompat::Auto, true),
            Some(true)
        );
        std::fs::write(path.join("shaders/final.fsh"), "void main() {}\n").unwrap();
        std::fs::write(path.join("shaders/final.vsh"), "void main() {}\n").unwrap();
        assert_eq!(
            super::pick_identity(&name, IrisCompat::Auto, true),
            Some(false)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Refused with the frame switch's refusal cache under On, the same files run again once
    /// the player picks Off: the frame no longer counts the pack as refused (`pack_shape`).
    #[test]
    fn a_refusal_under_one_switch_is_tried_again_under_another() {
        use crate::game::native_shaders::{fingerprint, outcome, remember};
        let root =
            std::env::temp_dir().join(format!("sunrise-iris-refused-{}", std::process::id()));
        let path = hdr_pack(&root, "refused", "");
        let mut settings = hdr_settings();
        let name = path.to_string_lossy().into_owned();
        settings.sunrise.shader_pack = name.clone();
        settings.sunrise.set_iris_compat(&name, IrisCompat::On);
        remember(&path, fingerprint(&path), IrisCompat::On, false);
        assert!(!pack_shape(&settings, true, &mut PickHdr::default()).draws_flat);
        settings.sunrise.set_iris_compat(&name, IrisCompat::Off);
        assert_eq!(outcome(&path, IrisCompat::Off), None);
        assert!(pack_shape(&settings, true, &mut PickHdr::default()).draws_flat);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The load's id files see the answers `read_properties` gives: a mapping guarded by a flag
    /// the pack lists resolves, one guarded by a flag it doesn't list doesn't, in
    /// `block.properties`, `entity.properties` and `item.properties` alike.
    #[test]
    fn the_loads_id_files_see_the_features_the_pack_lists() {
        use sunrise_shaders::{Macros, Pack};
        let root = std::env::temp_dir().join(format!("sunrise-id-features-{}", std::process::id()));
        let path = hdr_pack(&root, "ids", "iris.features.optional = SSBO\n");
        let guarded = |kind: &str, listed: &str, unlisted: &str| {
            format!(
                "#ifdef IRIS_FEATURE_SSBO\n{kind}.10 = {listed}\n#endif\n\
                 #ifdef IRIS_FEATURE_CUSTOM_IMAGES\n{kind}.11 = {unlisted}\n#endif\n"
            )
        };
        for (file, text) in [
            (
                "block",
                guarded("block", "minecraft:stone", "minecraft:dirt"),
            ),
            (
                "entity",
                guarded("entity", "minecraft:pig", "minecraft:cow"),
            ),
            (
                "item",
                guarded("item", "minecraft:stick", "minecraft:apple"),
            ),
        ] {
            std::fs::write(path.join(format!("shaders/{file}.properties")), text).unwrap();
        }
        let pack = Pack::open(&path).unwrap();
        let mut macros = Macros::standard();
        macros.define_renderer(4);
        sunrise_shaders::read_properties(&pack, &mut macros);
        let ids = read_id_files(&pack, &macros);
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(
            ids.blocks.id("minecraft:stone", |_| None, |_| false),
            Some(10)
        );
        assert_eq!(ids.blocks.id("minecraft:dirt", |_| None, |_| false), None);
        let entities = &ids.named.entities;
        assert_eq!(
            (entities.get("minecraft:pig"), entities.get("minecraft:cow")),
            (Some(&10), None)
        );
        let items = &ids.named.items;
        assert_eq!(
            (items.get("minecraft:stick"), items.get("minecraft:apple")),
            (Some(&10), None)
        );
        assert!(ids.named.item_file);
    }

    /// The kept request is read again when the pick changes and at each boundary that can
    /// change the pick's files or options (`PickHdr::forget`), not before: the same pack replaced
    /// under its name, and its options saved, give the new answer there.
    #[test]
    fn the_kept_hdr_request_is_read_again_at_its_boundaries() {
        let root = std::env::temp_dir().join(format!("sunrise-hdr-kept-{}", std::process::id()));
        let asks = hdr_pack(&root, "asks", "sunrise.hdr = true\n");
        let plain = hdr_pack(&root, "plain", "separateAo=true\n");
        let mut settings = hdr_settings();
        let mut state = PickHdr::default();
        let mut shape = |state: &mut PickHdr, path: &std::path::Path| {
            shape_of(&mut settings, state, path, true)
        };
        // The pick turned off, as the toggle does: no pack runs, under the switch.
        let settings_off = |state: &mut PickHdr| {
            let mut off = hdr_settings();
            off.sunrise.shader_pack.clear();
            assert!(!pack_shape(&off, true, state).draws_flat);
        };
        assert_eq!(shape(&mut state, &asks), (true, Some(Tonemap::Pack)));
        // Replaced under the same name: kept until a boundary, read there.
        std::fs::write(
            asks.join("shaders/shaders.properties"),
            "sunrise.hdr=false\n",
        )
        .unwrap();
        assert_eq!(shape(&mut state, &asks), (true, Some(Tonemap::Pack)));
        state.forget();
        assert_eq!(shape(&mut state, &asks), (false, None));
        // Another pick reads its own; back to this one, it is read again.
        std::fs::write(
            asks.join("shaders/shaders.properties"),
            "sunrise.hdr=true\n",
        )
        .unwrap();
        assert_eq!(shape(&mut state, &plain), (false, None));
        assert_eq!(shape(&mut state, &asks), (true, Some(Tonemap::Pack)));

        // Turned off (the toggle's empty pick), replaced under its name, turned on again: read
        // again.
        settings_off(&mut state);
        std::fs::write(asks.join("shaders/shaders.properties"), "x=1\n").unwrap();
        assert_eq!(shape(&mut state, &asks), (false, None));
        std::fs::write(
            asks.join("shaders/shaders.properties"),
            "sunrise.hdr=true\n",
        )
        .unwrap();
        settings_off(&mut state);
        assert_eq!(shape(&mut state, &asks), (true, Some(Tonemap::Pack)));

        // Behind the pack's own switch, off by default: asked once the saved options turn it
        // on, and off again when they turn it off, each at the save.
        let option = hdr_pack(
            &root,
            "option",
            "#ifdef HDR_OUTPUT\nsunrise.hdr=true\n#endif\n",
        );
        std::fs::write(
            option.join("shaders/settings.glsl"),
            "//#define HDR_OUTPUT\n#ifdef HDR_OUTPUT\n#endif\n",
        )
        .unwrap();
        assert_eq!(shape(&mut state, &option), (false, None));
        let chosen = root.join("option.txt");
        std::fs::write(&chosen, "HDR_OUTPUT=true\n").unwrap();
        assert_eq!(shape(&mut state, &option), (false, None));
        state.forget();
        assert_eq!(shape(&mut state, &option), (true, Some(Tonemap::Pack)));
        std::fs::write(&chosen, "HDR_OUTPUT=false\n").unwrap();
        state.forget();
        assert_eq!(shape(&mut state, &option), (false, None));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The load names the answer Sunrise serves the pack, which a frame kept after a failed
    /// rebuild doesn't decide: served nothing, a pack that asks says it is served `None`.
    #[test]
    fn the_load_names_the_served_answer() {
        use super::served_hdr;
        use sunrise_shaders::{Macros, Pack};
        let root = std::env::temp_dir().join(format!("sunrise-hdr-served-{}", std::process::id()));
        let asks = hdr_pack(&root, "asks", "sunrise.hdr = true\n");
        let pack = Pack::open(&asks).unwrap();
        // The frame was kept for another pick's request.
        let served = served_hdr(
            true,
            ("other", Some(Tonemap::Pack)),
            ("asks", Some(Tonemap::Pack)),
        );
        let name = asks.to_string_lossy();
        let (_, log) = loaded_properties(
            &pack,
            &mut Macros::standard(),
            true,
            served,
            &name,
            IrisCompat::Auto,
            &mut PickHdr::default(),
        );
        assert_eq!(
            log,
            [
                "shaders.properties asks for Sunrise's HDR as Some(Pack) with the renderer's \
                 macros, but it is served None: the served answer counts"
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The loader's read of the properties (`loaded_properties`, what a load logs), with the
    /// renderer's macros, after the frame was made for the pick's request. Where a renderer
    /// macro changes the request it says the served answer counts; an unknown value only the
    /// renderer's macros reach is noted once, with its line, key and value, and one the frame's
    /// read already noted is not noted again. Without the switch it logs what the file's own
    /// notes are, as before. The load reads the pack's files again at the next frame.
    #[test]
    fn the_loader_reads_the_request_again_with_the_renderers_macros() {
        use sunrise_shaders::{Macros, Pack};
        let root = std::env::temp_dir().join(format!("sunrise-hdr-loader-{}", std::process::id()));
        let renderer_curve = hdr_pack(
            &root,
            "renderer_curve",
            "sunrise.hdr=true\n#ifdef MC_GL_RENDERER_OTHER\nsunrise.hdr.tonemap=sunrise\n#endif\n",
        );
        let renderer_bad = hdr_pack(
            &root,
            "renderer_bad",
            "sunrise.hdr=true\n#ifdef MC_GL_RENDERER_OTHER\nsunrise.hdr.tonemap=maybe\n#endif\n",
        );
        let bad = hdr_pack(&root, "bad", "a=1\nsunrise.hdr = yes\n");
        let mut settings = hdr_settings();
        let mut state = PickHdr::default();
        let load = |state: &mut PickHdr, path: &std::path::Path, switch: bool, frame| {
            let pack = Pack::open(path).unwrap();
            let mut macros = Macros::standard();
            macros.define_renderer(4);
            let name = path.to_string_lossy();
            loaded_properties(
                &pack,
                &mut macros,
                switch,
                frame,
                &name,
                IrisCompat::Auto,
                state,
            )
            .1
        };
        let names_value = |line: &str, at: u32, key: &str, value: &str| {
            line.starts_with(&format!("shaders.properties:{at}: "))
                && line.contains(&format!("`{key}`"))
                && line.contains(&format!("`{value}`"))
        };

        // The frame is made for the standard macros' answer: the pack's curve.
        let frame = shape_of(&mut settings, &mut state, &renderer_curve, true).1;
        assert_eq!(frame, Some(Tonemap::Pack));
        let warned = load(&mut state, &renderer_curve, true, frame);
        assert_eq!(
            warned,
            [
                "shaders.properties asks for Sunrise's HDR as Some(Sunrise) with the renderer's \
                 macros, but it is served Some(Pack): the served answer counts"
            ]
        );
        assert!(load(&mut state, &renderer_curve, false, frame).is_empty());
        // A frame made for the loader's answer: nothing to say.
        assert!(load(&mut state, &renderer_curve, true, Some(Tonemap::Sunrise)).is_empty());

        // An unknown value behind a renderer macro: the frame's read can't see it, the load
        // notes it, once.
        let frame = shape_of(&mut settings, &mut state, &renderer_bad, true).1;
        assert_eq!(frame, Some(Tonemap::Pack));
        let noted = load(&mut state, &renderer_bad, true, frame);
        assert_eq!(noted.len(), 2, "{noted:?}");
        assert!(names_value(&noted[0], 3, "sunrise.hdr.tonemap", "maybe"));
        assert_eq!(load(&mut state, &renderer_bad, true, frame).len(), 1);
        assert!(load(&mut state, &renderer_bad, false, frame).is_empty());

        // One the frame's read noted is not noted again by the load.
        let frame = shape_of(&mut settings, &mut state, &bad, true).1;
        assert_eq!(frame, None);
        assert!(load(&mut state, &bad, true, frame).is_empty());

        // Without the switch the load logs what it logged before: each required feature
        // Sunrise lacks, then each note of the file, in that order; with it, the request's note
        // after them.
        let others = hdr_pack(
            &root,
            "others",
            "iris.features.required = SSBO NO_SUCH_FEATURE\nnot a pair\n\
             sunrise.hdr = maybe\n",
        );
        // The first names the flag, the second the file and line: not the English of either.
        let order_of = |logged: &[String]| {
            let flag = logged[0].split('`').nth(1).map(str::to_owned);
            (flag, logged[1].starts_with("shaders.properties:2:"))
        };
        let expected = (Some("NO_SUCH_FEATURE".to_owned()), true);
        let without_switch = load(&mut state, &others, false, None);
        assert_eq!(without_switch.len(), 2, "{without_switch:?}");
        assert_eq!(order_of(&without_switch), expected);
        let with_switch = load(&mut state, &others, true, None);
        assert_eq!(order_of(&with_switch), expected);
        assert_eq!(with_switch.len(), 3, "{with_switch:?}");
        assert!(names_value(&with_switch[2], 3, "sunrise.hdr", "maybe"));

        // The load reads the files again: the pack replaced under its name gives its new
        // request at the next frame.
        let frame = shape_of(&mut settings, &mut state, &renderer_curve, true);
        assert_eq!(frame, (true, Some(Tonemap::Pack)));
        std::fs::write(
            renderer_curve.join("shaders/shaders.properties"),
            "separateAo=true\n",
        )
        .unwrap();
        assert_eq!(
            shape_of(&mut settings, &mut state, &renderer_curve, true),
            frame
        );
        load(&mut state, &renderer_curve, true, frame.1);
        assert_eq!(
            shape_of(&mut settings, &mut state, &renderer_curve, true),
            (false, None)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Sunrise's HDR is served only under the switch, as the renderer's frame was made: then
    /// the pack's programs see `SUNRISE_HDR` 1, `SUNRISE_HDR_TONEMAP_PACK` with the pack's
    /// curve, and its `final` is translated for linear light; otherwise nothing changes.
    #[test]
    fn hdr_is_served_only_under_the_switch_as_the_frame_was_made() {
        use super::{serve_hdr, served_hdr};
        use sunrise_shaders::{GameInputs, Macros};
        let served = |switch, frame| {
            let mut macros = Macros::standard();
            let mut inputs = GameInputs::default();
            let hdr = served_hdr(switch, ("p", frame), ("p", frame));
            serve_hdr(hdr, &mut macros, &mut inputs);
            (
                macros.get("SUNRISE_HDR").map(str::to_owned),
                macros.is_defined("SUNRISE_HDR_TONEMAP_PACK"),
                inputs.hdr,
                macros == Macros::standard(),
            )
        };
        for frame in [None, Some(Tonemap::Pack), Some(Tonemap::Sunrise)] {
            assert_eq!(served(false, frame), (None, false, None, true), "{frame:?}");
        }
        assert_eq!(served(true, None), (None, false, None, true));
        assert_eq!(
            served(true, Some(Tonemap::Pack)),
            (Some("1".into()), true, Some(Tonemap::Pack), false)
        );
        assert_eq!(
            served(true, Some(Tonemap::Sunrise)),
            (Some("1".into()), false, Some(Tonemap::Sunrise), false)
        );
    }

    /// A frame that keeps serving takes the pick's new request; one made again keeps the one
    /// it was made for until the new one is made (`build_renderer` sets it then).
    #[test]
    fn a_kept_frame_takes_the_new_request_and_a_rebuild_keeps_the_old() {
        let (mut pick, mut installed) = (String::new(), None);
        let mut reconcile =
            |follows, picked| reconcile_pack_hdr(follows, (&mut pick, &mut installed), picked);
        assert!(!reconcile(true, ("a", Some(Tonemap::Pack))));
        assert!(!reconcile(true, ("a", Some(Tonemap::Sunrise))));
        assert!(reconcile(false, ("b", None)));
        assert_eq!((pick.as_str(), installed), ("a", Some(Tonemap::Sunrise)));
        assert!(!reconcile_pack_hdr(
            true,
            (&mut pick, &mut installed),
            ("b", None)
        ));
        assert_eq!((pick.as_str(), installed), ("b", None));
    }

    /// A frame kept after its rebuild failed (`reconcile_pack_hdr`: the frame doesn't follow)
    /// keeps the answer it was made for, but the loader serves it only to the pick it was made
    /// for, with that request: another pick that doesn't ask, the same pick replaced under its
    /// name by one that doesn't, or asks for the other curve, get nothing. Without the switch,
    /// nothing either.
    #[test]
    fn a_frame_kept_after_a_failed_rebuild_serves_only_the_pick_it_was_made_for() {
        use super::served_hdr;
        let (mut pick, mut installed) = (String::new(), None);
        assert!(!reconcile_pack_hdr(
            true,
            (&mut pick, &mut installed),
            ("opted", Some(Tonemap::Pack))
        ));
        let served = |pick: &str, installed, current| served_hdr(true, (pick, installed), current);
        assert_eq!(
            served(&pick, installed, ("opted", Some(Tonemap::Pack))),
            Some(Tonemap::Pack)
        );
        // Replaced by a pack that doesn't ask: its frame is to be made again, and that fails.
        assert!(reconcile_pack_hdr(
            false,
            (&mut pick, &mut installed),
            ("plain", None)
        ));
        assert_eq!(installed, Some(Tonemap::Pack));
        assert_eq!(served(&pick, installed, ("plain", None)), None);
        // An opted pack of another name, or this one asking otherwise now.
        assert_eq!(
            served(&pick, installed, ("other", Some(Tonemap::Pack))),
            None
        );
        assert_eq!(served(&pick, installed, ("opted", None)), None);
        assert_eq!(
            served(&pick, installed, ("opted", Some(Tonemap::Sunrise))),
            None
        );
        assert_eq!(
            served_hdr(false, (&pick, installed), ("opted", Some(Tonemap::Pack))),
            None
        );
    }

    #[test]
    fn native_shader_follows_its_mod_without_affecting_other_packs() {
        let mut mods = ModList::default();
        mods.installed.push(Installed {
            id: "sunrise_better_visuals".into(),
            title: "Better Visuals".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(NativePage::default()),
        });
        let owned = "native-sunrise_better_visuals";
        assert_eq!(active_pack(owned, &mods), owned);
        mods.disabled.insert("sunrise_better_visuals".into());
        assert_eq!(active_pack(owned, &mods), "");
        assert_eq!(active_pack("golden_hour", &mods), "golden_hour");
        mods.installed[0].native = None;
        assert_eq!(active_pack(owned, &mods), owned);
    }
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn projectile_body_uses_its_own_shader_entity_id() {
    use sunrise_sim::projectiles::{Projectile, ProjectileKind};

    super::native_entity_refs::tests::with_client_game(|game| {
        game.client.pack_ids = Some(PackIds::with_entity_ids([
            ("minecraft:snowball", 41),
            ("minecraft:ominous_item_spawner", 42),
            ("minecraft:arrow", 43),
            ("minecraft:spectral_arrow", 44),
        ]));
        let arrow = |effect| ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect,
        };
        for kind in [
            ProjectileKind::Snowball,
            arrow(None),
            arrow(Some(("minecraft:glowing", 200))),
        ] {
            game.creatures.projectiles.push(Projectile::new(
                kind,
                sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
                None,
            ));
        }
        game.entity_geometry(
            glam::DVec3::new(0.5, 64.0, -4.5),
            1.0,
            None,
            Vec::new(),
            &mut Vec::new(),
        );
        let ids: Vec<_> = game
            .client
            .entity_spans
            .borrow()
            .windows(2)
            .filter(|span| span[0].start < span[1].start && (41..=44).contains(&span[0].id))
            .map(|span| span[0].id)
            .collect();
        assert_eq!(ids, [41, 43, 44]);
    });
}

#[cfg(test)]
pub(in crate::game) mod toggle_tests {
    use super::{TogglePlan, plan_shader_toggle};
    use sunrise_client::settings::Settings;

    fn plan(pick: &str, last: &str, present: bool, active: bool) -> String {
        match plan_shader_toggle(pick, last, false, |_| present, |_| active) {
            TogglePlan::Off(name) => format!("off {name}"),
            TogglePlan::On(name) => format!("on {name}"),
            TogglePlan::HintModOff(name) => format!("mod-off {name}"),
            TogglePlan::HintClearLast => "hint-clear".into(),
            TogglePlan::Hint => "hint".into(),
            TogglePlan::Noop => "noop".into(),
        }
    }

    /// The key turns the picked pack off and remembers it, resumes it, and hints when there
    /// is nothing to resume; a removed pack clears the memory instead of failing, and a pack
    /// whose owning mod is off is reported instead of announced.
    #[test]
    fn the_toggle_turns_off_resumes_and_hints() {
        assert_eq!(plan("A.zip", "", true, true), "off A.zip");
        assert_eq!(plan("", "A.zip", true, true), "on A.zip");
        assert_eq!(plan("", "", true, true), "hint");
        assert_eq!(plan("", "A.zip", false, true), "hint-clear");
        assert_eq!(
            plan("", "native-frame_owner", true, false),
            "mod-off native-frame_owner"
        );
    }

    /// The developer override wins over the pick: the key does nothing.
    #[test]
    fn the_developer_override_makes_the_toggle_a_noop() {
        for (pick, last) in [("A.zip", ""), ("", "A.zip"), ("", "")] {
            assert!(
                matches!(
                    plan_shader_toggle(pick, last, true, |_| true, |_| true),
                    TogglePlan::Noop
                ),
                "pick {pick:?}, last {last:?}"
            );
        }
    }

    /// One shared pick: the menu's choice and the key's write the same fields, so turning
    /// off by either keeps the last non-empty pick for the key to resume.
    #[test]
    fn picking_and_unpicking_share_the_picks_memory() {
        let mut s = Settings::default();
        let pick_of = |s: &Settings| {
            (
                s.sunrise.shader_pack.clone(),
                s.extras.last_shader_pack.clone(),
            )
        };
        s.set_shader_pick("A.zip");
        assert_eq!(pick_of(&s), ("A.zip".into(), "A.zip".into()));
        // Picking another pack while turned off by the key forgets the older one.
        s.set_shader_pick("");
        s.set_shader_pick("B.zip");
        assert_eq!(pick_of(&s), ("B.zip".into(), "B.zip".into()));
        // Unpicking by hand keeps the memory, as the key's turning off does.
        s.set_shader_pick("");
        assert_eq!(pick_of(&s), ("".into(), "B.zip".into()));
    }

    /// A rapid double press ends where a single one does: off keeps the memory, on takes it
    /// back, and the loader (not the key) owns reinstalling the pack.
    #[test]
    fn a_rapid_double_press_restores_the_pick() {
        let mut s = Settings::default();
        s.set_shader_pick("sunrise:warm");
        s.set_shader_pick("");
        assert_eq!(
            (
                s.sunrise.shader_pack.as_str(),
                s.extras.last_shader_pack.as_str()
            ),
            ("", "sunrise:warm")
        );
        s.set_shader_pick(&s.extras.last_shader_pack.clone());
        assert_eq!(
            (
                s.sunrise.shader_pack.as_str(),
                s.extras.last_shader_pack.as_str()
            ),
            ("sunrise:warm", "sunrise:warm")
        );
    }
}

use sunrise_gameplay_common::game::weather::WeatherGameExt as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod named_id_tests {
    use super::NamedIds;
    use sunrise_shaders::{Macros, Pack};

    /// The ids of a pack folder `name` whose `shaders/` holds `files` (path, text), read as the
    /// loader reads them.
    fn named_ids_of(name: &str, files: &[(&str, &str)]) -> NamedIds {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(name);
        std::fs::create_dir_all(root.join("shaders")).unwrap();
        std::fs::write(root.join("shaders/shaders.properties"), "").unwrap();
        for (path, text) in files {
            std::fs::write(root.join("shaders").join(path), text).unwrap();
        }
        NamedIds::read(&Pack::open(&root).unwrap(), &Macros::standard()).0
    }

    /// 5.3: an entity `entity.properties` lacks reads 65535, and 0 when the pack has no
    /// `entity.properties`, whatever `item.properties` holds; held items follow their own file.
    #[test]
    fn the_missing_entity_id_follows_entity_properties_alone() {
        let entity_only = named_ids_of(
            "entity-only",
            &[("entity.properties", "entity.37=minecraft:zombie\n")],
        );
        assert_eq!(entity_only.entity_id("minecraft:zombie"), 37);
        assert_eq!(entity_only.entity_id("minecraft:creeper"), 65535);
        assert!(!entity_only.item_file);
        assert_eq!(entity_only.missing_item, 0);

        let item_only = named_ids_of(
            "item-only",
            &[("item.properties", "item.53=minecraft:stick\n")],
        );
        assert_eq!(item_only.entity_id("minecraft:creeper"), 0);
        assert_eq!(item_only.items.get("minecraft:stick"), Some(&53));
        assert!(item_only.item_file);
        assert_eq!(item_only.missing_item, -1);

        // An empty file is still the pack's file.
        let empty_entity = named_ids_of("empty-entity", &[("entity.properties", "")]);
        assert_eq!(empty_entity.entity_id("minecraft:creeper"), 65535);

        let both = named_ids_of(
            "both",
            &[
                ("entity.properties", "entity.37=minecraft:zombie\n"),
                ("item.properties", "item.53=minecraft:stick\n"),
            ],
        );
        assert_eq!(both.entity_id("minecraft:zombie"), 37);
        assert_eq!(both.entity_id("minecraft:creeper"), 65535);
        assert_eq!(both.missing_item, -1);

        let neither = named_ids_of("neither", &[]);
        assert_eq!(neither.entity_id("minecraft:creeper"), 0);
        assert!(!neither.item_file);
        assert_eq!(neither.missing_item, 0);
    }

    /// A zipped `entity.properties` or `item.properties` past the pack's 64 MiB read cap
    /// (`MAX_FILE` in `sunrise-shaders`' `pack.rs`) gives no ids, but the pack still has the
    /// file: an entity it lacks reads 65535 and a held item −1 (5.3).
    #[test]
    fn a_file_past_the_read_cap_still_counts_as_the_packs() {
        use std::io::Write;
        let oversized = |file: &str| {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            w.start_file("shaders/shaders.properties", options).unwrap();
            w.start_file(format!("shaders/{file}"), options).unwrap();
            let mut text = b"entity.37=minecraft:zombie\nitem.53=minecraft:stick\n".to_vec();
            text.resize((64 << 20) + 1, b' ');
            w.write_all(&text).unwrap();
            let bytes = w.finish().unwrap().into_inner();
            NamedIds::read(
                &Pack::from_zip_bytes("P.zip", bytes).unwrap(),
                &Macros::standard(),
            )
            .0
        };
        let entity = oversized("entity.properties");
        assert_eq!(entity.entity_id("minecraft:zombie"), 65535);
        assert_eq!(entity.entity_id("minecraft:creeper"), 65535);
        assert!(!entity.item_file);
        assert_eq!(entity.missing_item, 0);

        let item = oversized("item.properties");
        assert_eq!(item.entity_id("minecraft:creeper"), 0);
        assert!(item.items.is_empty());
        assert!(item.item_file);
        assert_eq!(item.missing_item, -1);
    }
}
