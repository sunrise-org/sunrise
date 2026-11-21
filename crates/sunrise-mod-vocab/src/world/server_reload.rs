//! Server data reload listeners and their registration context.
#[cfg(test)]
#[path = "server_reload_tests.rs"]
mod tests;

use crate::{
    FileToIdConverter, Identifier, ReloadResourceError,
    world::reload::{Boundary, ReloadCore, ReloadFiles},
    world::reload_order::ReloadOrderError,
};
use std::{
    any::Any,
    collections::BTreeMap,
    marker::PhantomData,
    sync::{Arc, LazyLock},
};

/// Sunrise's vanilla server reload listeners and ordering boundaries.
/// Use these methods to obtain IDs for server reload dependencies.
// no wire: namespace for local reload dependency IDs, not a payload.
#[non_exhaustive]
pub struct VanillaServerListeners;
impl VanillaServerListeners {
    /// Boundary before every reload listener.
    pub fn first() -> Identifier {
        static ID: LazyLock<Identifier> =
            LazyLock::new(|| Identifier::parse("minecraft:first").unwrap());
        ID.clone()
    }
    /// Vanilla's function library. Use this anchor rather than its string value,
    /// which need not match NeoForge's identifier.
    pub fn functions() -> Identifier {
        static ID: LazyLock<Identifier> =
            LazyLock::new(|| Identifier::parse("minecraft:function_library").unwrap());
        ID.clone()
    }
    /// Boundary after every reload listener.
    pub fn last() -> Identifier {
        static ID: LazyLock<Identifier> =
            LazyLock::new(|| Identifier::parse("minecraft:last").unwrap());
        ID.clone()
    }
}

/// Selected server data files for one converter, with pack overrides resolved.
/// Keys have their directory and extension removed. This cannot read assets or
/// filesystem paths. IDs and bytes are borrowed from this view.
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: a local prepare view of the winning server data files.
pub struct ServerReloadContext {
    files: ReloadFiles,
}
impl ServerReloadContext {
    pub(crate) fn new(files: ReloadFiles) -> Self {
        Self { files }
    }
    /// Converted IDs, across all namespaces, in deterministic resource-ID order.
    pub fn list(&self) -> impl Iterator<Item = &Identifier> {
        self.files.list()
    }
    /// The winning bytes for a converted ID, or None outside this directory.
    pub fn read(&self, id: &Identifier) -> Option<&[u8]> {
        self.files.read(id)
    }
}

/// A server-side `SimplePreparableReloadListener`: prepare reads the new pack
/// view without changing live state; the game consumes its returned action to
/// publish the result. All prepares finish before any apply, including vanilla's,
/// runs on the server's owning thread. Prepare cannot see another listener's new
/// published state. Dependencies order both phases; they do not remove the barrier.
/// On a prepare error, pending actions are dropped without running, keeping the
/// previous data and function library.
pub trait ServerReloadListener: Send + Sync + 'static {
    /// Parse and validate new data without changing the listener's live state.
    /// Return `Ok(Box::new(move || { /* publish prepared data */ }))` with owned
    /// data. The action is consumed once and cannot report an error. A later
    /// server table refusal leaves the previous engine generation in place but
    /// does not undo effects of actions that already ran.
    ///
    /// As vanilla's `SimpleJsonResourceReloadListener.scanDirectory`, a
    /// listener that parses its files logs and skips one that won't parse,
    /// carrying the rest through; returning `Err` aborts the whole reload and
    /// keeps every listener's previous data. Attach the original error to the
    /// file's ID with [`ReloadResourceError::new`] to preserve its cause.
    /// A panic in `prepare` or the returned apply stops the server; the crash
    /// report names the listener and the mod that registered it.
    fn prepare(
        &self,
        context: &ServerReloadContext,
    ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError>;
}
impl ServerReloadListener for Boundary {
    fn prepare(
        &self,
        _: &ServerReloadContext,
    ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
        Ok(Box::new(|| {}))
    }
}

/// The server reload's listener registry, as `add_server_reload_listeners` hands it out: its
/// registration methods and a read-only view of names and edges, rebuilt on each server reload.
/// Listener names need not belong to the registering mod.
pub trait ServerReloadListeners {
    /// Register a unique listener and converter, e.g. `FileToIdConverter::json("machines")`.
    /// Keep the `Arc` to read the listener's published state outside reloads.
    fn add_listener(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ServerReloadListener>,
    ) -> Result<(), ReloadOrderError>;
    /// [`add_listener`](Self::add_listener), keeping `retained` (the listener itself, as the
    /// mod holds it) in the reload resources; mods reach it through `add_retained_listener`.
    fn add_retained(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ServerReloadListener>,
        retained: Arc<dyn Any + Send + Sync>,
    ) -> Result<(), ReloadOrderError>;
    /// Require `first` before `second`; both names must already be registered.
    fn add_dependency(
        &mut self,
        first: Identifier,
        second: Identifier,
    ) -> Result<(), ReloadOrderError>;
    /// Borrow the registered names in registration order, boundaries included.
    fn get_registry(&self) -> ServerReloadRegistryView<'_>;
    /// Borrow the dependency graph as `(first, second)` names.
    fn get_graph(&self) -> ServerReloadGraphView<'_>;
    /// Borrow the effective vanilla anchor preceding mod listeners by default.
    /// On the server this is the built-in loot modifier listener once registered,
    /// not the function library. Use `VanillaServerListeners::functions()` to
    /// order against the function library itself.
    fn get_last_vanilla_listener(&self) -> &Identifier;
}

// no wire: borrows the live server reload registry during registration.
/// The names registered so far in one server reload, borrowed from the registry.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ServerReloadRegistryView<'a> {
    core: &'a ReloadCore<dyn ServerReloadListener>,
}

impl<'a> ServerReloadRegistryView<'a> {
    /// Iterate over registered names in registration order, boundaries included.
    pub fn iter(&self) -> impl Iterator<Item = &'a Identifier> + use<'a> {
        self.core.registry().map(|(id, _)| id)
    }
}

// no wire: borrows the live server reload dependency graph during registration.
/// The current server reload dependencies, borrowed from the registry.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ServerReloadGraphView<'a> {
    core: &'a ReloadCore<dyn ServerReloadListener>,
}

impl<'a> ServerReloadGraphView<'a> {
    /// Iterate over dependency edges in insertion order. Each `(first, second)`
    /// edge requires `first` to run before `second`; the edges include the
    /// built-in ordering boundaries and are not a sorted list of listeners.
    pub fn iter(&self) -> impl Iterator<Item = (&'a Identifier, &'a Identifier)> + use<'a> {
        self.core.graph()
    }
}

impl dyn ServerReloadListeners + '_ {
    /// Register a listener whose object is also kept in the reload resources, as
    /// NeoForge's `addRetainedListener(key, listener)`: the caller builds the
    /// typed `ListenerKey` (`ListenerKey::new(id)`), and reads the object back
    /// through `ServerReloadResources` after the reload, from any server hook.
    pub fn add_retained_listener<T: ServerReloadListener>(
        &mut self,
        key: &ListenerKey<T>,
        converter: FileToIdConverter,
        listener: Arc<T>,
    ) -> Result<(), ReloadOrderError> {
        self.add_retained(key.id().clone(), converter, listener.clone(), listener)
    }
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`ServerReloadListeners`].
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ServerListenerRegistry {
    core: ReloadCore<dyn ServerReloadListener>,
    retained: Vec<RetainedEntry>,
    pub(crate) default_anchor: Identifier,
}
impl ServerReloadListeners for ServerListenerRegistry {
    fn add_listener(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ServerReloadListener>,
    ) -> Result<(), ReloadOrderError> {
        self.core.add_listener(id, converter, listener)
    }
    fn add_retained(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<dyn ServerReloadListener>,
        retained: Arc<dyn Any + Send + Sync>,
    ) -> Result<(), ReloadOrderError> {
        self.core.add_listener(id.clone(), converter, listener)?;
        self.retained.push(RetainedEntry { id, any: retained });
        Ok(())
    }
    fn add_dependency(
        &mut self,
        first: Identifier,
        second: Identifier,
    ) -> Result<(), ReloadOrderError> {
        self.core.add_dependency(first, second)
    }
    fn get_registry(&self) -> ServerReloadRegistryView<'_> {
        ServerReloadRegistryView { core: &self.core }
    }
    fn get_graph(&self) -> ServerReloadGraphView<'_> {
        ServerReloadGraphView { core: &self.core }
    }
    fn get_last_vanilla_listener(&self) -> &Identifier {
        &self.default_anchor
    }
}
impl ServerListenerRegistry {
    pub(crate) fn new() -> Self {
        Self {
            core: ReloadCore::new(
                VanillaServerListeners::first(),
                VanillaServerListeners::functions(),
                VanillaServerListeners::last(),
                || -> Arc<dyn ServerReloadListener> { Arc::new(Boundary) },
            ),
            retained: Vec::new(),
            default_anchor: VanillaServerListeners::functions(),
        }
    }
    pub(crate) fn finish(self) -> Result<OrderedServerReload, ReloadOrderError> {
        Ok(OrderedServerReload {
            listeners: self.core.finish(&self.default_anchor)?,
            retained: self.retained,
        })
    }
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The ordered listeners and retained entries of one completed registration.
#[non_exhaustive]
pub struct OrderedServerReload {
    /// The listeners in dependency order.
    pub listeners: Vec<crate::world::reload::RegisteredListener<dyn ServerReloadListener>>,
    /// The entries the reload keeps.
    pub retained: Vec<RetainedEntry>,
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// One listener retained in the reload resources, as NeoForge's
/// `ReloadableServerResources` keeps a manager.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RetainedEntry {
    pub(crate) id: Identifier,
    pub(crate) any: Arc<dyn Any + Send + Sync>,
}

// no wire: typed lookup key for process-local retained trait objects; phantom type identity is not transferable.
/// A typed key to a retained reload listener, built by the registering mod
/// and read back through `ReloadResources`. `ListenerKey::new` accepts any
/// id; a key whose id does not match a registration, or whose type differs
/// from the registered object, reads back as `None`.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ListenerKey<T: ?Sized> {
    id: Identifier,
    marker: PhantomData<fn() -> T>,
}
impl<T: ?Sized> ListenerKey<T> {
    /// Names a listener key.
    pub fn new(id: Identifier) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }
    /// The registered listener's name.
    pub fn id(&self) -> &Identifier {
        &self.id
    }
}
impl<T: ?Sized> Clone for ListenerKey<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            marker: PhantomData,
        }
    }
}
impl<T: ?Sized> std::fmt::Debug for ListenerKey<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListenerKey").field("id", &self.id).finish()
    }
}
impl<T: ?Sized> PartialEq for ListenerKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<T: ?Sized> Eq for ListenerKey<T> {}

/// The retained server reload listeners of the last successful reload, as
/// NeoForge's `ReloadableServerResources`, reached from
/// `ServerHost::reload_resources`. A new reload replaces the whole store.
pub trait ServerReloadResources {
    /// The object retained under `id`, as its registering mod handed it over; mods read it
    /// through `listener`.
    fn retained(&self, id: &Identifier) -> Option<Arc<dyn Any + Send + Sync>>;
}
impl dyn ServerReloadResources + '_ {
    /// The retained listener registered under `key` in the last successful
    /// reload; `None` when nothing is registered under it, its type differs, or
    /// no reload has succeeded yet. The downcast itself checks the type.
    pub fn listener<T: ServerReloadListener>(&self, key: &ListenerKey<T>) -> Option<Arc<T>> {
        self.retained(key.id())?.downcast::<T>().ok()
    }
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`ServerReloadResources`].
#[derive(Default)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RetainedListeners {
    entries: BTreeMap<Identifier, Arc<dyn Any + Send + Sync>>,
}
impl ServerReloadResources for RetainedListeners {
    fn retained(&self, id: &Identifier) -> Option<Arc<dyn Any + Send + Sync>> {
        self.entries.get(id).cloned()
    }
}
impl RetainedListeners {
    pub(crate) fn install(&mut self, retained: Vec<RetainedEntry>) {
        self.entries = retained
            .into_iter()
            .map(|entry| (entry.id, entry.any))
            .collect();
    }
}
