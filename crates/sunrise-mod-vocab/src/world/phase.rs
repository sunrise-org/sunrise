//! Restricted views for registration and data loading.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The owning boundary that starts a data load, as reported by [`DataLoadContext::origin`].
// no wire: identifies the owner of a local data load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataLoadOrigin {
    /// Server data loading, including a reload.
    ServerDataLoad,
    /// Client resource preloading.
    ClientPreload,
}

/// The side of the local resource-generation loop, as reported by [`LoopContext::side`].
// no wire: identifies the owner of a local loop, not an installation or join contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoopSide {
    /// A server loop.
    Server,
    /// A client loop.
    Client,
}

/// The permission boundary of the current owning thread. Only the game advances it.
// no wire: tracks the owning thread's phase in this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModPhase {
    /// Package discovery, before declarations.
    Packages,
    /// Once-only declarations, without a Host.
    Registration,
    /// One side's startup, before its configs and world.
    Setup,
    /// Config loading: settings reads and scoped config files.
    Config,
    /// The owning game's normal runtime.
    World,
    /// Restricted data or asset loading, including published resource queries,
    /// returning to its entering phase.
    Reloading,
}

/// A refused engine transition never changes the phase or dispatches a hook.
// no wire: reports a failed transition to the local phase owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PhaseError {
    /// The token belongs to another thread.
    WrongThread,
    /// The requested edge is not part of the phase machine.
    Transition {
        /// Unchanged current phase.
        from: ModPhase,
        /// Refused destination.
        to: ModPhase,
    },
}
impl std::fmt::Display for PhaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mod phase transition refused: {self:?}")
    }
}
impl std::error::Error for PhaseError {}

// no wire: plain phase snapshot for the engine-owned load-state socket.
// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.
#[doc(hidden)]
#[derive(Clone, Copy)]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
pub struct PhaseState {
    pub phase: ModPhase,
    pub resume: Option<ModPhase>,
}
// no wire: thread-bound phase authority or RAII restoration guard; never transferred as data.
/// Engine-only authority for one thread; moving it does not transfer authority.
#[derive(Clone)]
#[non_exhaustive]
pub struct PhaseOwner {
    thread: std::thread::ThreadId,
    fallback: Arc<std::sync::Mutex<PhaseState>>,
}
impl PhaseOwner {
    /// Bind a new owning thread (or a loader) to its initial phase. Registration
    /// is process-owned; a new side starts at Setup after those tables are frozen.
    pub fn new(initial: ModPhase) -> Self {
        let state = PhaseState {
            phase: initial,
            resume: None,
        };
        let owner = Self {
            thread: std::thread::current().id(),
            fallback: Arc::new(std::sync::Mutex::new(state)),
        };
        owner.write_state(state);
        owner
    }
    fn read_state(&self) -> PhaseState {
        crate::dispatch_scope::backend().map_or_else(
            || {
                *self
                    .fallback
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
            },
            |backend| (backend.phase_state)(),
        )
    }
    fn write_state(&self, state: PhaseState) {
        if let Some(backend) = crate::dispatch_scope::backend() {
            (backend.set_phase)(state);
        } else {
            *self
                .fallback
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = state;
        }
    }
    /// A loader's temporary thread-local phase; the calling thread's state is restored.
    pub fn scope(initial: ModPhase) -> PhaseScope {
        let previous = crate::dispatch_scope::backend().map_or(
            PhaseState {
                phase: current_phase(),
                resume: None,
            },
            |backend| (backend.phase_state)(),
        );
        PhaseScope {
            owner: Self::new(initial),
            previous,
            _thread: std::marker::PhantomData,
        }
    }
    /// Advance a legal edge on the bound thread, without dispatching anything.
    pub fn advance(&self, to: ModPhase) -> Result<(), PhaseError> {
        if self.thread != std::thread::current().id() {
            return Err(PhaseError::WrongThread);
        }
        {
            let mut state = self.read_state();
            let from = state.phase;
            match (from, to) {
                (ModPhase::Packages, ModPhase::Registration)
                | (ModPhase::Registration, ModPhase::Setup)
                | (ModPhase::Setup, ModPhase::Config)
                | (ModPhase::Config, ModPhase::World)
                | (ModPhase::World, ModPhase::Config) => {}
                (ModPhase::Setup | ModPhase::World, ModPhase::Reloading) => {
                    state.resume = Some(from);
                }
                (ModPhase::Reloading, _) if state.resume == Some(to) => {
                    state.resume = None;
                }
                _ => return Err(PhaseError::Transition { from, to }),
            }
            state.phase = to;
            self.write_state(state);
            Ok(())
        }
    }
    /// Load configs before returning to normal runtime, including on failure.
    pub fn config(&self) -> Result<PhaseGuard<'_>, PhaseError> {
        self.advance(ModPhase::Config)?;
        Ok(PhaseGuard {
            owner: self,
            resume: ModPhase::World,
            _thread: std::marker::PhantomData,
        })
    }
    /// Nest a reload in Setup or World; errors and panics restore the caller's phase.
    pub fn reloading(&self) -> Result<PhaseGuard<'_>, PhaseError> {
        let resume = self.read_state().phase;
        self.advance(ModPhase::Reloading)?;
        Ok(PhaseGuard {
            owner: self,
            resume,
            _thread: std::marker::PhantomData,
        })
    }
}

// no wire: thread-bound phase authority or RAII restoration guard; never transferred as data.
/// A temporary loader context, not authority transferred to a worker.
// exhaustive: engine-only constructor and private state
pub struct PhaseScope {
    owner: PhaseOwner,
    previous: PhaseState,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl std::ops::Deref for PhaseScope {
    type Target = PhaseOwner;
    fn deref(&self) -> &PhaseOwner {
        &self.owner
    }
}
impl Drop for PhaseScope {
    fn drop(&mut self) {
        self.owner.write_state(self.previous);
    }
}

// no wire: thread-bound phase authority or RAII restoration guard; never transferred as data.
/// Engine-only phase scope, restored during unwinding too.
// exhaustive: only the engine constructs it
pub struct PhaseGuard<'a> {
    owner: &'a PhaseOwner,
    resume: ModPhase,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for PhaseGuard<'_> {
    fn drop(&mut self) {
        self.owner
            .advance(self.resume)
            .expect("scoped phase returns to its declared destination");
    }
}

/// The current owning phase, or the initial phase before the platform installs its reader.
pub fn current_phase() -> ModPhase {
    crate::dispatch_scope::backend().map_or(
        if cfg!(test) {
            ModPhase::World
        } else {
            ModPhase::Packages
        },
        |backend| (backend.current_phase)(),
    )
}
/// Whether this thread is in the once-only registration phase.
///
/// Use this in a scripting binding before accepting declarations: a script called, for example,
/// from `Hooks::player_logged_in` is too late. This read does not grant a registrar or reopen
/// registration. Only the relevant registration hook can submit its declarations.
pub fn registration_open() -> bool {
    current_phase() == ModPhase::Registration
}

/// The catalog-owned access class of a generated game operation.
// no wire: the generated phase gate reads this access class in the same process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadAccess {
    /// No extra load access beyond the phase's metadata/settings exceptions.
    WorldOnly,
    /// Registry and immutable metadata available in setup and reload.
    Metadata,
    /// The existing client setup reads, not general reload permissions.
    ClientSetup,
    /// Published asset/data facts, including resource-cache queries.
    PublishedResources,
}

/// The default body of a host method: a test double that does not implement it gets the access
/// check, then a panic naming the method.
pub(crate) fn unprovided(method: &'static str, operation: &'static str, access: LoadAccess) -> ! {
    check_access(operation, access);
    unimplemented!("{}", method)
}

/// One cell read at the generated API boundary, not a tick scan or registry lookup; in
/// `World` nothing more.
///
/// A refused call records a [`PhaseRefusal`](super::phase_refusal::PhaseRefusal) in the
/// thread's containment scope and unwinds with it (`resume_unwind`, so no panic hook and no
/// crash report). The dispatcher that called the mod catches it with `contain_refusal`, drops
/// that listener's answer and goes on. The scope latches: while it holds a refusal every later
/// gated call in it refuses too, whatever its access class, and the refusal is reported even
/// if the mod caught the unwind and returned normally.
///
/// Two cases are not contained. When the thread is already unwinding (a mod `Drop` running
/// during a panic, say) the gate logs the named line and panics, and a panic during an unwind
/// aborts the process. With no containment scope on the thread it panics with the named
/// message; no production path reaches that.
#[inline]
pub fn check_access(operation: &'static str, access: LoadAccess) {
    let phase = current_phase();
    if phase != ModPhase::World
        && (!phase_allows(phase, operation, access) || super::phase_refusal::latched())
    {
        super::phase_refusal::refuse(operation, phase);
    }
}

/// Whether an operation is available in this thread's current load phase.
#[inline]
pub fn access_allowed(operation: &str, access: LoadAccess) -> bool {
    phase_allows(current_phase(), operation, access)
}

#[inline]
fn phase_allows(phase: ModPhase, operation: &str, access: LoadAccess) -> bool {
    if phase == ModPhase::World {
        return true;
    }
    let metadata = access == LoadAccess::Metadata;
    let settings = operation == "setting";
    match phase {
        ModPhase::World => true,
        ModPhase::Setup => {
            settings
                || metadata
                || matches!(
                    access,
                    LoadAccess::ClientSetup | LoadAccess::PublishedResources
                )
        }
        ModPhase::Config => settings || operation == "config_files",
        ModPhase::Reloading => settings || metadata || access == LoadAccess::PublishedResources,
        ModPhase::Packages | ModPhase::Registration => false,
    }
}

/// Whether a loaded package contains compiled code or content alone.
// no wire: describes an entry in the local package inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PackageKind {
    /// The package carries compiled code.
    Code,
    /// The package carries content alone: data and assets.
    Content,
}

/// Read-only metadata from the local package inventory, in dispatch order through [`LoadedMods`].
/// Read it directly; to send or store package facts, copy the fields you need into your own record.
///
/// ```compile_fail,E0277
/// use mod_api::{PackageInfo, wire::Writer};
/// fn write_package(writer: &mut Writer, package: &PackageInfo) {
///     writer.put(package);
/// }
/// ```
// no wire: this snapshot is for inspecting packages in the current process.
#[derive(Clone)]
#[non_exhaustive]
pub struct PackageInfo {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) kind: PackageKind,
    pub(crate) side: crate::Side,
    /// Its own namespace, then the others it declares.
    pub(crate) namespaces: Vec<String>,
    /// Its `[dependencies]` rows, sorted by id.
    pub(crate) dependencies: Vec<crate::PackageDependency>,
    pub(crate) details: crate::PackageDetails,
}

impl PackageInfo {
    /// An owned package snapshot supplied by the loader.
    pub fn new(
        id: String,
        version: String,
        kind: PackageKind,
        side: crate::Side,
        namespaces: Vec<String>,
        dependencies: Vec<crate::PackageDependency>,
    ) -> Self {
        Self {
            id,
            version,
            kind,
            side,
            namespaces,
            dependencies,
            details: crate::PackageDetails::default(),
        }
    }
    /// The same snapshot with the manifest's descriptive fields.
    pub fn with_details(mut self, details: crate::PackageDetails) -> Self {
        self.details = details;
        self
    }
    /// Package identifier.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// The package's version.
    pub fn version(&self) -> &str {
        &self.version
    }
    /// Whether this package contains code.
    pub fn kind(&self) -> PackageKind {
        self.kind
    }
    /// The package's declared side.
    pub fn side(&self) -> crate::Side {
        self.side
    }
    /// The package's own namespace followed by any additional namespaces.
    pub fn namespaces(&self) -> impl Iterator<Item = &str> {
        self.namespaces.iter().map(String::as_str)
    }
    /// Whether this package declares a dependency on `id`.
    pub fn depends_on(&self, id: &str) -> bool {
        self.dependencies.iter().any(|d| d.id() == id)
    }
    /// Its declared dependencies, sorted by id.
    pub fn dependencies(&self) -> &[crate::PackageDependency] {
        &self.dependencies
    }
    /// The display name; empty when the snapshot carries no manifest details.
    pub fn name(&self) -> &str {
        &self.details.name
    }
    /// The description, or empty.
    pub fn description(&self) -> &str {
        &self.details.description
    }
    /// The authors, in manifest order.
    pub fn authors(&self) -> &[String] {
        &self.details.authors
    }
    /// The license name, or empty.
    pub fn license(&self) -> &str {
        &self.details.license
    }
    /// The home page link.
    pub fn homepage(&self) -> Option<&str> {
        self.details.homepage.as_deref()
    }
    /// Where to report problems.
    pub fn issues(&self) -> Option<&str> {
        self.details.issues.as_deref()
    }
    /// The icon's path inside the package, such as `icon.png`.
    pub fn icon_path(&self) -> Option<&str> {
        self.details.icon_path.as_deref()
    }
    /// One value of its manifest's `[properties]` table.
    pub fn property(&self, key: &str) -> Option<&crate::PropertyValue> {
        self.details.properties.get(key)
    }
}

// no wire: shared immutable loader inventory; contexts query it through LoadReader rather than transferring its Arc storage.
/// The instance directory, registries and settings that registration reads: captured by the
/// engine once, before the first registration hook, and the same for every registration step.
/// Registry and settings lookups are binary searches over sorted lists.
#[derive(Clone)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RegistrationData {
    instance_dir: Arc<Path>,
    runtime: crate::RuntimeEnvironment,
    /// Built-in registry names in ID order, by registry name.
    pub(crate) registries: Arc<[(String, Arc<[String]>)]>,
    /// Each loaded mod's settings, by mod ID then key.
    pub(crate) settings: Arc<[(String, Arc<[(String, String)]>)]>,
    /// The same load-owned behavior owners that supply the game's default providers.
    capability_coverage: Option<fn(crate::ProvidedBy, &str, &str) -> bool>,
    block_entity_coverage: Option<fn(&str, &str) -> bool>,
    /// Each package's checked runtime files, by mod ID.
    runtime_files: Arc<std::collections::BTreeMap<String, Arc<dyn crate::RuntimeFileSource>>>,
}

impl RegistrationData {
    /// The engine's snapshot: `registries` maps a registry name to its names in ID order,
    /// `settings` a mod ID to its settings, and `instance_dir` is the startup game directory.
    pub fn new(
        registries: BTreeMap<String, Vec<String>>,
        settings: BTreeMap<String, BTreeMap<String, String>>,
        instance_dir: PathBuf,
        runtime: crate::RuntimeEnvironment,
    ) -> Self {
        Self {
            instance_dir: instance_dir.into(),
            runtime,
            capability_coverage: None,
            block_entity_coverage: None,
            runtime_files: Default::default(),
            registries: registries
                .into_iter()
                .map(|(name, ids)| (name, ids.into()))
                .collect(),
            settings: settings
                .into_iter()
                .map(|(id, values)| (id, values.into_iter().collect()))
                .collect(),
        }
    }
    /// The engine's initial load shares the published registration settings
    /// rather than capturing the same startup files again on its workers.
    /// Without a registration publication, the load owns a fresh capture.
    pub fn initial_settings_snapshot() -> Option<Arc<[(String, Arc<[(String, String)]>)]>> {
        registration_data().map(|data| data.settings)
    }
    /// Bind each package's checked runtime files, by mod ID: what
    /// [`RegistrationContext::runtime_files`] serves the mod being dispatched.
    pub fn with_runtime_files(
        mut self,
        files: std::collections::BTreeMap<String, Arc<dyn crate::RuntimeFileSource>>,
    ) -> Self {
        self.runtime_files = Arc::new(files);
        self
    }
    pub(crate) fn runtime_source(&self, mod_id: &str) -> Option<Arc<dyn crate::RuntimeFileSource>> {
        self.runtime_files.get(mod_id).cloned()
    }
    /// Bind the game's pure type-coverage projection. It reads no world or runtime position.
    pub fn with_capability_coverage(
        mut self,
        coverage: fn(crate::ProvidedBy, &str, &str) -> bool,
        block_entity_coverage: fn(&str, &str) -> bool,
    ) -> Self {
        self.capability_coverage = Some(coverage);
        self.block_entity_coverage = Some(block_entity_coverage);
        self
    }
    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        let at = self
            .registries
            .binary_search_by(|(known, _)| known.as_str().cmp(name));
        Some(self.registries[at.ok()?].1.clone())
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        let mod_at = self
            .settings
            .binary_search_by(|(known, _)| known.as_str().cmp(mod_id))
            .ok()?;
        let values = &self.settings[mod_at].1;
        let at = values
            .binary_search_by(|(known, _)| known.as_str().cmp(key))
            .ok()?;
        Some(values[at].1.clone())
    }
}

fn registration_data() -> Option<RegistrationData> {
    crate::dispatch_scope::backend().and_then(|backend| (backend.registration_data)())
}

pub(crate) fn game_capability_registered(by: crate::ProvidedBy, name: &str, id: &str) -> bool {
    if matches!(by, crate::ProvidedBy::Item)
        && crate::parts::installation::builtin_item_capability_registered(name, id)
    {
        return true;
    }
    registration_data()
        .and_then(|data| data.capability_coverage)
        .is_some_and(|coverage| coverage(by, name, id))
}

pub(crate) fn block_entity_covers(kind: &str, block: &str) -> bool {
    registration_data()
        .and_then(|data| data.block_entity_coverage)
        .is_some_and(|coverage| coverage(kind, block))
}
/// Read-only view of the startup package inventory, shared: cloning it copies no package.
// no wire: the loader publishes the inventory locally; callers borrow or clone the view.
#[derive(Clone)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct LoadedMods {
    pub(crate) entries: Arc<[PackageInfo]>,
}

impl LoadedMods {
    pub(crate) fn new() -> Self {
        #[cfg(test)]
        if let Some(entries) = TEST_PACKAGES.with_borrow(Clone::clone) {
            return Self { entries };
        }
        Self {
            entries: crate::dispatch_scope::backend()
                .map_or_else(|| Arc::from([]), |backend| (backend.packages)()),
        }
    }

    /// Package IDs in dispatch order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(PackageInfo::id)
    }

    /// Packages in dispatch order.
    pub fn iter(&self) -> impl Iterator<Item = &PackageInfo> {
        self.entries.iter()
    }

    /// Find a package by ID. This linear lookup is intended for registration, not hot paths.
    pub fn get(&self, id: &str) -> Option<&PackageInfo> {
        self.entries.iter().find(|entry| entry.id() == id)
    }
}

/// Why a read through [`RegistrationContext`] was refused. Handle this error at the call site.
// no wire: reports failure of a local registration read.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegistrationReadError {
    /// The engine supplied no startup snapshot to this context.
    Unavailable,
    /// The registry name is not a namespaced identifier (`minecraft:item`).
    InvalidRegistryName(String),
    /// No built-in registry has this name.
    UnknownRegistry(String),
    /// The setting key is empty or names a path rather than one key.
    InvalidSettingKey(String),
    /// No loaded package has this ID, so it has no settings to read.
    UnknownMod(String),
}
impl std::fmt::Display for RegistrationReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "registration read refused: {self:?}")
    }
}
impl std::error::Error for RegistrationReadError {}

/// A registration phase's view. Declarations go on the hook's event; there is
/// no world, handle constructor, transaction or game Host at this point.
///
/// Every registration hook (`register`, `component_types`, `blocks`, `items`,
/// `standalone_models` and the other `Register*` hooks) reads the same frozen view,
/// captured once before the first of them runs, so an answer never depends on
/// dispatch order or on which hook asks:
///
/// - [`mods`](Self::mods): the loaded packages.
/// - [`registry`](Self::registry): the built-in registries, as `registries.json` lists them.
///   Entries that mods declare during registration are not in it; a mod already has its own
///   declarations on its event or registrar.
/// - [`setting`](Self::setting): the settings on disk for the loaded packages when registration
///   began. Later writes are not seen.
///
/// In [`Hooks::register`](crate::Hooks::register), this context also records [`warn`](Self::warn)
/// and [`refuse`](Self::refuse) diagnostics for the mod being registered.
///
/// ```compile_fail
/// use mod_api::{Host, RegistrationContext};
/// fn game(_: &dyn Host) {}
/// fn registration(context: &RegistrationContext) { game(context); }
/// ```
///
/// ```compile_fail
/// use mod_api::{prelude::EntitiesHost, RegistrationContext};
/// fn registration(context: &RegistrationContext) { let _ = EntitiesHost::entity(context, 1); }
/// ```
///
/// ```compile_fail
/// use mod_api::{HostTransaction, RegistrationContext};
/// fn registration(context: &mut RegistrationContext) {
///     let _: Result<(), ()> = context.transaction(|_, _| Ok(()));
/// }
/// ```
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: the phase context owns local registration state and the startup screen.
pub struct RegistrationContext {
    pub(crate) mods: LoadedMods,
    pub(crate) data: Option<RegistrationData>,
    pub(crate) declaration_refusals: std::sync::Mutex<Vec<crate::__private::DeclarationRefusal>>,
    pub(crate) diagnostics:
        Option<Arc<std::sync::Mutex<crate::__private::RegistrationDiagnostics>>>,
    /// The game's startup screen, for [`progress`](Self::progress) and
    /// [`message`](Self::message); none without a game.
    pub(crate) startup: Option<Arc<dyn crate::StartupScreen>>,
    pub(crate) runtime: Option<crate::RuntimeEnvironment>,
}

impl RegistrationContext {
    pub(crate) fn new() -> Self {
        let data = registration_data();
        let runtime = data.as_ref().map(|data| data.runtime.clone());
        Self {
            mods: LoadedMods::new(),
            data,
            declaration_refusals: Default::default(),
            diagnostics: None,
            startup: crate::world::startup_progress::installed(),
            runtime,
        }
    }

    /// Physical process, independent of the logical side of a hook.
    pub fn dist(&self) -> crate::Dist {
        self.runtime().dist()
    }
    /// Exact Minecraft version captured before registration.
    pub fn game_version(&self) -> &str {
        self.runtime().game_version()
    }
    /// Sunrise version, identical to the crash report's version.
    pub fn sunrise_version(&self) -> &str {
        self.runtime().sunrise_version()
    }
    /// Startup opt-in for test-only content; does not gate test registration.
    pub fn game_tests_enabled(&self) -> bool {
        self.runtime().game_tests_enabled()
    }
    fn runtime(&self) -> &crate::RuntimeEnvironment {
        self.runtime
            .as_ref()
            .expect("registration runtime environment is installed")
    }

    /// Borrow the instance directory captured before registration, preserving non-Unicode names.
    /// Use [`Path::to_path_buf`] to keep it beyond this context. The path gives no filesystem
    /// authority. Returns [`RegistrationReadError::Unavailable`] if there is no startup snapshot.
    pub fn instance_dir(&self) -> Result<&Path, RegistrationReadError> {
        Ok(self.data()?.instance_dir.as_ref())
    }

    /// The loaded-mod view, independent of this phase's event or registrar.
    pub fn mods(&self) -> &LoadedMods {
        &self.mods
    }

    /// A built-in registry's names in ID order, for validating the keys a mod declares.
    pub fn registry(&self, name: &str) -> Result<Arc<[String]>, RegistrationReadError> {
        if crate::Identifier::parse_namespaced(name).is_none() {
            return Err(RegistrationReadError::InvalidRegistryName(name.to_owned()));
        }
        self.data()?
            .registry(name)
            .ok_or_else(|| RegistrationReadError::UnknownRegistry(name.to_owned()))
    }

    /// A loaded mod's setting as it was when registration began: `Ok(None)` when the mod
    /// has not set it. The key is one setting's name, not a path.
    pub fn setting(
        &self,
        mod_id: &str,
        key: &str,
    ) -> Result<Option<String>, RegistrationReadError> {
        if key.is_empty() || matches!(key, "." | "..") || key.contains(['/', '\\', '\0']) {
            return Err(RegistrationReadError::InvalidSettingKey(key.to_owned()));
        }
        let data = self.data()?;
        if self.mods.get(mod_id).is_none() {
            return Err(RegistrationReadError::UnknownMod(mod_id.to_owned()));
        }
        Ok(data.setting(mod_id, key))
    }

    fn data(&self) -> Result<&RegistrationData, RegistrationReadError> {
        self.data.as_ref().ok_or(RegistrationReadError::Unavailable)
    }
}

/// A data load's read-only view. The loader freezes the metadata for this
/// resource generation before dispatch. The metadata getters lend that data
/// without file I/O. Copy data only if it must outlive the context borrow.
/// Reading already-loaded data never creates this context.
///
/// ```compile_fail
/// use mod_api::{DataLoadContext, Host};
/// fn game(_: &dyn Host) {}
/// fn load(context: &dyn DataLoadContext) { game(context); }
/// ```
///
/// ```compile_fail
/// use mod_api::{prelude::EntitiesHost, DataLoadContext};
/// fn load(context: &dyn DataLoadContext) { let _ = EntitiesHost::entity(context, 1); }
/// ```
///
/// ```compile_fail
/// use mod_api::{DataLoadContext, HostTransaction};
/// fn load(context: &mut dyn DataLoadContext) {
///     let _: Result<(), ()> = context.transaction(|_, _| Ok(()));
/// }
/// ```
pub trait DataLoadContext {
    /// The phase selected at the owning load boundary.
    fn origin(&self) -> DataLoadOrigin;

    /// Borrows registry names in ID order, or returns `None` if the registry
    /// is unavailable.
    fn registry(&self, name: &str) -> Option<&[String]>;

    /// Resolve a fluid name against this load's registered fluid identities.
    fn fluid_key(&self, id: &str) -> Option<crate::FluidKey>;

    /// Borrows the recipe pass's fluid tags (`data/<ns>/tags/fluid`), with each
    /// tag's members in source order. `None` means the load has no recipe
    /// overlay; `Some(&[])` means the overlay has no fluid tags.
    fn fluid_tags(&self) -> Option<&[(String, Arc<[String]>)]> {
        None
    }

    /// Borrows a captured setting, or returns `None` if it was not captured.
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str>;

    /// Borrows the loaded-mod inventory in dispatch order.
    fn mods(&self) -> &LoadedMods;
}

/// Immutable resource-generation reads supplied by an owning loader. Return
/// borrows into the captured data; the getters must not read files.
pub trait LoadReader: Sync {
    /// Borrows registry names in ID order, or returns `None` if unavailable.
    fn registry(&self, name: &str) -> Option<&[String]>;
    /// Borrows recipe fluid tags with members in source order. Return `None`
    /// without a recipe overlay and `Some(&[])` when it has no fluid tags.
    fn fluid_tags(&self) -> Option<&[(String, Arc<[String]>)]>;
    /// Borrows a captured setting, or returns `None` if it was not captured.
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str>;
}

// no wire: borrowed phase-scoped host readers; authority stays with their owning thread.
/// The game's owning load, as a [`DataLoadContext`].
// hidden: the game's side of the context, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private; only the game builds it
pub struct OwningLoad<'a> {
    source: &'a dyn LoadReader,
    origin: DataLoadOrigin,
    mods: LoadedMods,
}

impl<'a> OwningLoad<'a> {
    /// Borrow a loader's immutable reader at its owning boundary.
    pub fn new(origin: DataLoadOrigin, source: &'a dyn LoadReader) -> Self {
        Self {
            source,
            origin,
            mods: LoadedMods::new(),
        }
    }
}

impl DataLoadContext for OwningLoad<'_> {
    fn origin(&self) -> DataLoadOrigin {
        self.origin
    }
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.source.registry(name)
    }
    fn fluid_key(&self, id: &str) -> Option<crate::FluidKey> {
        crate::content::registry_keys::fluid_key(id)
    }
    fn fluid_tags(&self) -> Option<&[(String, Arc<[String]>)]> {
        self.source.fluid_tags()
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.source.setting(mod_id, key)
    }
    fn mods(&self) -> &LoadedMods {
        &self.mods
    }
}

/// The frozen resource-generation view of a hook fired inside a menu, recipe or
/// player-list loop: the side, registries, settings, mod list and authoritative item
/// tags. A loop without a tag-bearing snapshot (for example, the sound engine) returns
/// `None` from tag reads even after world tags sync. The side that owns the loop builds
/// this view; its answers never change while held. Metadata getters return borrows
/// tied to this context and do no file I/O.
///
/// ```compile_fail
/// use mod_api::{Host, LoopContext};
/// fn game(_: &dyn Host) {}
/// fn each(context: &dyn LoopContext) { game(context); }
/// ```
///
/// ```compile_fail
/// use mod_api::{HostTransaction, LoopContext};
/// fn each(context: &mut dyn LoopContext) {
///     let _: Result<(), ()> = context.transaction(|_, _| Ok(()));
/// }
/// ```
pub trait LoopContext {
    /// Membership in this generation's authoritative item tags. `None` means the
    /// snapshot is unavailable or a member of this tag could not be bound. Absent
    /// or empty tags in a loaded snapshot answer `Some(false)`. Reads allocate
    /// nothing and perform no file I/O.
    fn is_in_tag(&self, item: crate::ItemKey, tag: crate::TagKey) -> Option<bool>;

    /// Borrow the bound item members in authoritative order for this generation.
    /// `None` means the snapshot was unavailable or a member of this tag could not
    /// be bound at publication; no partial list is returned. A missing or empty tag
    /// in a loaded snapshot returns `Some(&[])`. On clients these are the server's
    /// synced tags, never the local pack's. Reads allocate nothing.
    fn items_in_tag(&self, tag: crate::TagKey) -> Option<&[crate::ItemKey]>;

    /// Whether a mod-created recipe property set accepts an item ID. Unknown
    /// sets/items and vanilla set keys return false. Use this in
    /// `menu_slot_rules` for the same filter on server and client prediction.
    /// Answers come from the owning load, replaced by the server's synced
    /// generation on clients; no allocation or file IO occurs during a read.
    fn mod_recipe_property_contains(&self, set: &str, item: crate::ItemKey) -> bool;

    /// The side whose loop fires the hook.
    fn side(&self) -> LoopSide;

    /// Borrows registry names in ID order, or returns `None` if the registry
    /// is unavailable.
    fn registry(&self, name: &str) -> Option<&[String]>;

    /// Borrows a captured setting, or returns `None` if it was not captured.
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str>;

    /// Borrows the loaded-mod inventory in dispatch order.
    fn mods(&self) -> &LoadedMods;
}

#[path = "loop_item_tags.rs"]
mod loop_item_tags;
pub use loop_item_tags::BoundItemTags;

/// Immutable resource-generation reads supplied by an owning loop. The getters
/// borrow the captured metadata and must not read files. Item tags are bound
/// separately when the generation is published.
pub trait LoopReader: Sync {
    /// Mod-created recipe property membership from the owning generation.
    fn mod_recipe_property_contains(&self, set: &str, item: crate::ItemKey) -> bool {
        let _ = (set, item);
        false
    }
    /// Borrows registry names in ID order, or returns `None` if unavailable.
    fn registry(&self, name: &str) -> Option<&[String]>;
    /// Borrows a captured setting, or returns `None` if it was not captured.
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str>;
}

// no wire: borrowed phase-scoped host readers; authority stays with their owning thread.
/// The game's loop, as a [`LoopContext`].
// hidden: the game's side of the context, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private; only the game builds it
pub struct OwningLoop<'a> {
    source: &'a dyn LoopReader,
    item_tags: Option<&'a BoundItemTags>,
    side: LoopSide,
    mods: LoadedMods,
}

impl<'a> OwningLoop<'a> {
    /// Borrow a loop's immutable reader and its already-bound tags at the owning boundary.
    pub fn new(
        side: LoopSide,
        source: &'a dyn LoopReader,
        item_tags: Option<&'a BoundItemTags>,
    ) -> Self {
        Self {
            source,
            item_tags,
            side,
            mods: LoadedMods::new(),
        }
    }
}

impl LoopContext for OwningLoop<'_> {
    fn is_in_tag(&self, item: crate::ItemKey, tag: crate::TagKey) -> Option<bool> {
        self.item_tags?.contains(tag, item)
    }
    fn items_in_tag(&self, tag: crate::TagKey) -> Option<&[crate::ItemKey]> {
        self.item_tags?.items(tag)
    }
    fn mod_recipe_property_contains(&self, set: &str, item: crate::ItemKey) -> bool {
        self.source.mod_recipe_property_contains(set, item)
    }
    fn side(&self) -> LoopSide {
        self.side
    }
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.source.registry(name)
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.source.setting(mod_id, key)
    }
    fn mods(&self) -> &LoadedMods {
        &self.mods
    }
}

/// The view of a hook fired while a feature generates, where no game is at
/// hand: on a worldgen worker thread, or on the server thread when a sapling
/// grows. It lends the server's registries, settings and mod list from the
/// frozen resource generation. The metadata borrows are tied to this host
/// and the getters do no file I/O. There are no writes, no level or entity
/// handles and no transactions: the hook's arguments provide access to the
/// generating region.
///
/// ```compile_fail
/// use mod_api::{Host, WorldgenHost};
/// fn game(_: &dyn Host) {}
/// fn generate(host: &dyn WorldgenHost) { game(host); }
/// ```
///
/// ```compile_fail
/// use mod_api::{HostTransaction, WorldgenHost};
/// fn generate(host: &mut dyn WorldgenHost) {
///     let _: Result<(), ()> = host.transaction(|_, _| Ok(()));
/// }
/// ```
pub trait WorldgenHost {
    /// Borrows registry names in ID order, or returns `None` if the registry
    /// is unavailable.
    fn registry(&self, name: &str) -> Option<&[String]>;

    /// Borrows a captured setting, or returns `None` if it was not captured.
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str>;

    /// Borrows the loaded-mod inventory in dispatch order.
    fn mods(&self) -> &LoadedMods;

    /// `BlockState.setValue`: `state` with its property `key` set to `value`; `state` itself
    /// when it already has that value, None when its block has no such property or value.
    fn state_with_property(
        &self,
        state: crate::BlockState,
        key: &str,
        value: &str,
    ) -> Option<crate::BlockState>;
}

// no wire: a process-local borrow of frozen worldgen metadata.
/// Immutable world-generation reads supplied by the owning engine boundary.
// hidden: engine plumbing, re-exported through __private for the game.
#[doc(hidden)]
pub trait WorldgenReader: Sync {
    /// Borrows registry names in ID order.
    fn registry(&self, name: &str) -> Option<&[String]>;
    /// Borrows a captured setting, without file I/O.
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str>;
}

// no wire: borrowed phase-scoped host readers; authority stays with their owning thread.
/// The server's world generation, as a [`WorldgenHost`].
// hidden: the game's side of the context, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private; only the game builds it
pub struct OwningWorldgen<'a> {
    source: &'a dyn WorldgenReader,
    state_with_property: &'a StateWithProperty<'a>,
    mods: LoadedMods,
}

/// `BlockState.setValue` as the game answers it for [`OwningWorldgen`].
type StateWithProperty<'a> =
    dyn Fn(crate::BlockState, &str, &str) -> Option<crate::BlockState> + 'a;

impl<'a> OwningWorldgen<'a> {
    /// Borrow the owning engine's frozen metadata reader and block-state query.
    pub fn new(
        source: &'a dyn WorldgenReader,
        state_with_property: &'a StateWithProperty<'a>,
    ) -> Self {
        Self {
            source,
            state_with_property,
            mods: LoadedMods::new(),
        }
    }
}

impl WorldgenHost for OwningWorldgen<'_> {
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.source.registry(name)
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.source.setting(mod_id, key)
    }
    fn mods(&self) -> &LoadedMods {
        &self.mods
    }
    fn state_with_property(
        &self,
        state: crate::BlockState,
        key: &str,
        value: &str,
    ) -> Option<crate::BlockState> {
        (self.state_with_property)(state, key, value)
    }
}

#[cfg(test)]
thread_local! {
    static TEST_PACKAGES: std::cell::RefCell<Option<Arc<[PackageInfo]>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_test_package_inventory<R>(
    packages: Vec<PackageInfo>,
    f: impl FnOnce() -> R,
) -> R {
    let previous = TEST_PACKAGES.replace(Some(packages.into()));
    let result = f();
    TEST_PACKAGES.set(previous);
    result
}

#[cfg(test)]
pub(crate) use crate::testing::TestReader;

#[test]
fn phase_authority_advances_without_a_platform_backend() {
    let owner = PhaseOwner::new(ModPhase::Packages);
    owner.advance(ModPhase::Registration).unwrap();
    let other = owner.clone();
    assert_eq!(
        std::thread::spawn(move || other.advance(ModPhase::Setup))
            .join()
            .unwrap(),
        Err(PhaseError::WrongThread)
    );
    owner.advance(ModPhase::Setup).unwrap();
    {
        let _reload = owner.reloading().unwrap();
    }
    owner.advance(ModPhase::Config).unwrap();
    owner.advance(ModPhase::World).unwrap();
    {
        let _config = owner.config().unwrap();
    }
    assert_eq!(
        owner.advance(ModPhase::Setup),
        Err(PhaseError::Transition {
            from: ModPhase::World,
            to: ModPhase::Setup
        })
    );
}

#[test]
fn the_registration_context_lists_installed_packages_with_their_details() {
    let details = crate::PackageDetails {
        name: "Machines".to_owned(),
        properties: BTreeMap::from([("ratio".to_owned(), crate::PropertyValue::Float(0.5))]),
        ..Default::default()
    };
    let packages = vec![
        PackageInfo::new(
            "machines".to_owned(),
            "2".to_owned(),
            PackageKind::Code,
            crate::Side::Both,
            vec!["machines".to_owned()],
            vec![crate::PackageDependency::new(
                "base".to_owned(),
                "^1".to_owned(),
                crate::DependencyKind::Required,
                crate::DependencyOrder::Before,
                crate::Side::Both,
                None,
                None,
            )],
        )
        .with_details(details),
        PackageInfo::new(
            "base".to_owned(),
            "1".to_owned(),
            PackageKind::Content,
            crate::Side::Client,
            vec!["base".to_owned()],
            vec![],
        ),
    ];
    with_test_package_inventory(packages, || {
        let context = RegistrationContext::new();
        assert_eq!(
            context.mods().ids().collect::<Vec<_>>(),
            ["machines", "base"]
        );
        let machines = context.mods().get("machines").unwrap();
        assert_eq!(machines.name(), "Machines");
        assert_eq!(machines.kind(), PackageKind::Code);
        assert_eq!(
            machines.property("ratio"),
            Some(&crate::PropertyValue::Float(0.5))
        );
        assert!(machines.depends_on("base"));
        assert_eq!(
            machines.dependencies()[0].order(),
            crate::DependencyOrder::Before
        );
        let base = context.mods().get("base").unwrap();
        assert_eq!(base.kind(), PackageKind::Content);
        assert!(context.mods().get("missing").is_none());
    });
}
