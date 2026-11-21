//! Shared reload listener registration, prepare/apply, errors and crash attribution.
//! `server_reload` and `client_reload` define each side's context, listener trait
//! and vanilla anchors.
use crate::{
    FileToIdConverter, Identifier,
    world::reload_order::{ReloadOrder, ReloadOrderError},
};
use std::{collections::BTreeMap, sync::Arc};

/// An error loading a resource, with its ID and the original cause available
/// through [`Self::resource`] and [`std::error::Error::source`].
// no wire: retains the native error and its source chain in this process.
#[derive(Debug)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ReloadResourceError {
    resource: Identifier,
    source: Box<dyn std::error::Error + Send + Sync>,
}
impl ReloadResourceError {
    /// Attach the resource ID to a parsing or validation error without losing
    /// its type. For a validation message alone, use `std::io::Error::other(message)`.
    pub fn new(
        resource: Identifier,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            resource,
            source: Box::new(source),
        }
    }

    /// The converted ID of the resource that failed to load.
    pub fn resource(&self) -> &Identifier {
        &self.resource
    }
}
impl std::fmt::Display for ReloadResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.resource, self.source)
    }
}
impl std::error::Error for ReloadResourceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// The stateless boundary listener each side's anchors are built from; each
/// side's file implements its listener trait for it.
pub(crate) struct Boundary;

/// Crash attribution for one listener call. A returned `Err` is recovered by the
/// reload; a panic in `prepare` or `apply` is fatal, and the report names the
/// registering mod (through the native-mod mark) and the listener (through the
/// report's details).
pub(crate) struct ReloadMark {
    _guard: Option<crate::content::crash::ModGuard>,
}

impl ReloadMark {
    pub(crate) fn enter(
        owner: Option<&'static str>,
        listener: &Identifier,
        phase: &'static str,
    ) -> Self {
        let _guard = owner.map(|id| {
            crate::content::crash::enter_native_mod(crate::content::crash::ModCall {
                id,
                hook: phase,
                part: None,
            })
        });
        crate::content::crash::set_detail("Reload listener", listener.to_string());
        Self { _guard }
    }
}

impl Drop for ReloadMark {
    fn drop(&mut self) {
        crate::content::crash::clear_detail("Reload listener");
    }
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// One registered listener of either side, between registration and ordering.
#[non_exhaustive]
pub struct RegisteredListener<L: ?Sized> {
    /// The listener's name.
    pub id: Identifier,
    /// Maps the listener's files to ids, if it reads a folder.
    pub converter: Option<FileToIdConverter>,
    /// The listener.
    pub listener: Arc<L>,
    /// The registering mod's id, for crash attribution on its later calls; the
    /// vanilla anchors have none.
    pub owner: Option<&'static str>,
}

/// Registration both sides share: unique names and objects, dependencies, the
/// first/anchor/last boundaries, failure recording, and the ordering sort. A
/// side adds what its moment needs (the server retains listeners) around it.
pub(crate) struct ReloadCore<L: ?Sized> {
    order: ReloadOrder<RegisteredListener<L>>,
    failure: Option<ReloadOrderError>,
    first: Identifier,
    last: Identifier,
}

impl<L: ?Sized + 'static> ReloadCore<L> {
    /// The `first`, vanilla-anchor and `last` boundaries with their default
    /// edges; `boundary` builds each id's own stateless listener, as the
    /// vanilla anchors are distinct listener objects.
    pub(crate) fn new(
        first: Identifier,
        anchor: Identifier,
        last: Identifier,
        boundary: impl Fn() -> Arc<L>,
    ) -> Self {
        let mut order = ReloadOrder::default();
        for id in [first.clone(), anchor.clone(), last.clone()] {
            order
                .add_listener(
                    id.clone(),
                    RegisteredListener {
                        id,
                        converter: None,
                        listener: boundary(),
                        owner: None,
                    },
                )
                .unwrap();
        }
        let mut core = Self {
            order,
            failure: None,
            first,
            last,
        };
        // The anchor keeps its place between the boundaries even though no
        // listener ever orders against it explicitly.
        core.order
            .add_dependency(core.first.clone(), anchor.clone())
            .unwrap();
        core.order
            .add_dependency(anchor, core.last.clone())
            .unwrap();
        core
    }
    /// Register a unique listener and converter, e.g.
    /// `FileToIdConverter::json("machines")`.
    pub(crate) fn add_listener(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<L>,
    ) -> Result<(), ReloadOrderError> {
        let owner = crate::content::crash::current_native_mod();
        let result = self.insert(id, converter, listener, owner);
        if let Err(error) = &result {
            self.failure.get_or_insert_with(|| error.clone());
        }
        result
    }
    fn insert(
        &mut self,
        id: Identifier,
        converter: FileToIdConverter,
        listener: Arc<L>,
        owner: Option<&'static str>,
    ) -> Result<(), ReloadOrderError> {
        if let Some(registered_as) = self.find_name(&*listener) {
            return Err(ReloadOrderError::DuplicateObject {
                id,
                registered_as: registered_as.clone(),
            });
        }
        self.order.add_listener(
            id.clone(),
            RegisteredListener {
                id: id.clone(),
                converter: Some(converter),
                listener,
                owner,
            },
        )?;
        self.order
            .add_dependency(self.first.clone(), id.clone())
            .unwrap();
        self.order.add_dependency(id, self.last.clone()).unwrap();
        Ok(())
    }
    /// Require `first` before `second`; both names must already be registered.
    pub(crate) fn add_dependency(
        &mut self,
        first: Identifier,
        second: Identifier,
    ) -> Result<(), ReloadOrderError> {
        let result = self.order.add_dependency(first, second);
        if let Err(error) = &result {
            self.failure.get_or_insert_with(|| error.clone());
        }
        result
    }
    /// Registration order, boundaries included.
    pub(crate) fn registry(&self) -> impl Iterator<Item = (&Identifier, &RegisteredListener<L>)> {
        self.order.registry()
    }
    /// Read-only dependency edges as `(first, second)` names.
    pub(crate) fn graph(&self) -> impl Iterator<Item = (&Identifier, &Identifier)> {
        self.order.graph()
    }
    /// Find a listener object's registered name, or None for an unregistered
    /// object.
    pub(crate) fn find_name(&self, listener: &L) -> Option<&Identifier> {
        self.registry().find_map(|(id, entry)| {
            std::ptr::addr_eq(entry.listener.as_ref(), listener).then_some(id)
        })
    }
    /// The ordered listeners, or the first registration failure; `anchor` is
    /// the vanilla listener unrelated mods sort after.
    pub(crate) fn finish(
        self,
        anchor: &Identifier,
    ) -> Result<Vec<RegisteredListener<L>>, ReloadOrderError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        self.order.sort(Some(anchor))
    }
}

/// A failed reload: staging its inputs failed, listeners could not be ordered,
/// or a listener's `prepare` returned an error. Both sides report listeners alike.
// no wire: local staging preserves the typed ordering and listener errors.
#[derive(Debug)]
#[non_exhaustive]
pub enum ReloadStageError {
    /// The next resource generation or its enabled-package inventory could not be built.
    DataSource(String),
    /// An accepted root's original jars or input record could not be read.
    OriginalJars(crate::JarInputsError),
    /// The listeners could not be ordered.
    Order(ReloadOrderError),
    /// A listener's `prepare` failed.
    Listener {
        /// The failing listener's name.
        listener: Identifier,
        /// Why it failed.
        source: ReloadResourceError,
    },
}
impl std::fmt::Display for ReloadStageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DataSource(error) => f.write_str(error),
            Self::OriginalJars(error) => error.fmt(f),
            Self::Order(error) => error.fmt(f),
            Self::Listener { listener, source } => {
                write!(f, "reload listener {listener}: {source}")
            }
        }
    }
}
impl std::error::Error for ReloadStageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::DataSource(_) => return None,
            Self::OriginalJars(error) => error,
            Self::Order(error) => error,
            Self::Listener { source, .. } => source,
        })
    }
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// One mod listener's prepared step, between its `prepare` and its `apply`.
/// The server's vanilla function library rides along as the `Vanilla` step of
/// its own reload; the client's has none.
#[non_exhaustive]
pub enum PreparedStep<V = ()> {
    /// A mod listener's prepared step.
    Mod {
        /// Publishes the prepared result, consuming the action.
        apply: Box<dyn FnOnce() + Send + 'static>,
        /// The owning mod's id, or `None` for the game's own.
        owner: Option<&'static str>,
        /// The listener's name.
        id: Identifier,
    },
    /// The server's vanilla function library step.
    Vanilla(V),
}

/// Orders the registered listeners, then prepares each with a converter
/// against the reload's own view: the shared loop of both sides' reloads.
/// `context_of` builds the side's context for one listener from its converter,
/// `prepare_of` calls the side's listener trait, and `vanilla` turns a
/// converter-less vanilla anchor into its own step (the server's function
/// library); the boundaries prepare nothing. Runs on the side's staging thread.
/// If a prepare fails, the already prepared actions are dropped without running.
pub fn prepare_listeners<L: ?Sized, C, V>(
    ordered: Vec<RegisteredListener<L>>,
    phase: &'static str,
    vanilla: impl Fn(&RegisteredListener<L>) -> Option<V>,
    context_of: impl Fn(&Identifier, &FileToIdConverter) -> C,
    prepare_of: impl Fn(&L, &C) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError>,
) -> Result<Vec<PreparedStep<V>>, ReloadStageError> {
    let mut prepared = Vec::new();
    for listener in ordered {
        if let Some(step) = vanilla(&listener) {
            prepared.push(PreparedStep::Vanilla(step));
            continue;
        }
        // The boundaries have no converter and no work.
        let Some(converter) = &listener.converter else {
            continue;
        };
        let context = context_of(&listener.id, converter);
        let _mark = ReloadMark::enter(listener.owner, &listener.id, phase);
        let apply = prepare_of(&*listener.listener, &context).map_err(|source| {
            ReloadStageError::Listener {
                listener: listener.id.clone(),
                source,
            }
        })?;
        prepared.push(PreparedStep::Mod {
            apply,
            owner: listener.owner,
            id: listener.id,
        });
    }
    Ok(prepared)
}

/// Consumes the prepared steps in order on the thread that owns the game,
/// attributing each mod listener's apply to its registering mod. `vanilla`
/// publishes the side's own step.
pub fn apply_listeners<V>(
    prepared: Vec<PreparedStep<V>>,
    phase: &'static str,
    mut vanilla: impl FnMut(V),
) {
    for step in prepared {
        match step {
            PreparedStep::Mod { apply, owner, id } => {
                let _mark = ReloadMark::enter(owner, &id, phase);
                apply();
            }
            PreparedStep::Vanilla(step) => vanilla(step),
        }
    }
}

/// The files one converter selected from a reload's packs, shared by both
/// sides' contexts; each side keeps its own context type so a wrong-side read
/// does not compile.
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: owns the local bytes the prepare context lends to a listener.
pub struct ReloadFiles(BTreeMap<Identifier, Vec<u8>>);
impl ReloadFiles {
    pub(crate) fn new(files: BTreeMap<Identifier, Vec<u8>>) -> Self {
        Self(files)
    }
    /// Converted IDs, across all namespaces, in deterministic resource-ID order.
    pub(crate) fn list(&self) -> impl Iterator<Item = &Identifier> {
        self.0.keys()
    }
    /// The winning bytes for a converted ID, or None outside this directory.
    pub(crate) fn read(&self, id: &Identifier) -> Option<&[u8]> {
        self.0.get(id).map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        num::{IntErrorKind, ParseIntError},
        sync::Mutex,
        thread,
    };

    #[test]
    fn reload_resource_error_borrows_the_resource_id() {
        let resource = Identifier::parse("test:machines/press").unwrap();
        let error =
            ReloadResourceError::new(resource.clone(), std::io::Error::other("invalid file"));

        assert_eq!(error.resource(), &resource);
    }

    #[test]
    fn reload_resource_error_exposes_the_typed_source() {
        let resource = Identifier::parse("test:machines/press").unwrap();
        let cause = "invalid".parse::<u32>().unwrap_err();
        let error = ReloadResourceError::new(resource, cause);

        let source = std::error::Error::source(&error).unwrap();
        let cause = source.downcast_ref::<ParseIntError>().unwrap();
        assert_eq!(cause.kind(), &IntErrorKind::InvalidDigit);
    }

    #[test]
    fn reload_moves_send_only_actions_to_the_applying_thread_in_order() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let applying_thread = thread::current().id();
        let prepared = thread::scope(|scope| {
            scope
                .spawn(|| {
                    assert_ne!(thread::current().id(), applying_thread);
                    let ordered = ["first", "vanilla", "last"]
                        .into_iter()
                        .map(|name| RegisteredListener {
                            id: Identifier::parse(&format!("test:{name}")).unwrap(),
                            converter: (name != "vanilla")
                                .then(|| FileToIdConverter::json("data").unwrap()),
                            listener: Arc::new(name),
                            owner: None,
                        })
                        .collect();
                    prepare_listeners(
                        ordered,
                        "reload_prepare",
                        |entry| entry.converter.is_none().then_some("apply vanilla"),
                        |_, _| (),
                        |&name, _| {
                            trace.lock().unwrap().push(format!("prepare {name}"));
                            let payload = Cell::new(format!("apply {name}"));
                            let trace = trace.clone();
                            Ok(Box::new(move || {
                                assert_eq!(thread::current().id(), applying_thread);
                                trace.lock().unwrap().push(payload.into_inner());
                            }))
                        },
                    )
                    .unwrap()
                })
                .join()
                .unwrap()
        });

        assert_eq!(
            trace.lock().unwrap().as_slice(),
            ["prepare first", "prepare last"]
        );
        apply_listeners(prepared, "reload_apply", |vanilla| {
            trace.lock().unwrap().push(vanilla.into());
        });
        assert_eq!(
            trace.lock().unwrap().as_slice(),
            [
                "prepare first",
                "prepare last",
                "apply first",
                "apply vanilla",
                "apply last"
            ]
        );
    }
}
