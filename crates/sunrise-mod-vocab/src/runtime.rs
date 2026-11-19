//! The installed mods, who is subscribed to what, and the crash marks around a call.
use crate::*;

// no wire: installed hook trait objects and shared dispatch ownership are process-local.
/// One native mod, with its bundle's `[mod] id` and hook implementation.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ModEntry {
    pub(crate) id: &'static str,
    pub(crate) hooks: Box<dyn Hooks>,
    /// Whether the game loaded it as a wasm module: what its registrations are checked against,
    /// whatever the module says of itself.
    pub(crate) wasm: bool,
}

impl ModEntry {
    /// Bind a hook implementation to its nonempty bundle id.
    pub fn new(id: &'static str, hooks: Box<dyn Hooks>) -> Self {
        assert!(!id.is_empty(), "native mod id must not be empty");
        Self {
            id,
            hooks,
            wasm: false,
        }
    }

    /// Bind a loaded wasm module's hook proxy to its bundle id.
    // hidden: the wasm runtime's side, not part of the mod surface
    #[doc(hidden)]
    pub fn wasm(id: &'static str, hooks: Box<dyn Hooks>) -> Self {
        Self {
            wasm: true,
            ..Self::new(id, hooks)
        }
    }

    /// Marks the thread as inside this mod's `hook` until the guard drops, so a panic in it
    /// names both. A trait injection's generated dispatch calls it.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn enter(&self, hook: &'static str) -> crate::content::crash::ModGuard {
        mark_mod(self.id, hook)
    }

    /// The first client-side hook this mod subscribes to, by name: the game refuses a
    /// `side = "server"` mod that has one.
    pub(crate) fn client_hook(&self) -> Option<&'static str> {
        client_hook_name(self.hooks.subscribed())
    }
}

/// The first client-side hook among `subscribed`, by name, in catalog order.
pub(crate) fn client_hook_name(subscribed: &[Option<HookId>]) -> Option<&'static str> {
    generated::CLIENT_HOOKS
        .iter()
        .find(|(id, _)| {
            subscribed
                .iter()
                .flatten()
                .any(|s| *s as usize == *id as usize)
        })
        .map(|(_, name)| *name)
}

// no wire: installed hook trait objects and shared dispatch ownership are process-local.
/// The ordered list of named mods shared by the client and server.
pub type Mods = Arc<[ModEntry]>;

pub(crate) struct Installed {
    pub(crate) mods: Mods,
    by_id: std::collections::HashMap<&'static str, usize>,
    pub(crate) subscribers: sunrise_mod_runtime::subscribers::Subscribers,
    pub(crate) disabled: Box<[std::sync::atomic::AtomicBool]>,
    mod_id_owners: std::collections::HashMap<&'static str, usize>,
    content_owners: OnceLock<std::collections::HashMap<Box<str>, Option<usize>>>,
    disabled_count: std::sync::atomic::AtomicUsize,
    /// The poison sets of the hooks whose dispatch isolates every listener
    /// (#2414 ruling): a listener that unwound is skipped by that one hook
    /// for the rest of the session. One set per hook, filled only on a panic;
    /// `poison_free` answers through one atomic load, so a dispatch with no
    /// poison pays nothing.
    poisons: std::sync::Mutex<Option<Box<[Option<std::collections::HashSet<&'static str>>]>>>,
    poison_free: std::sync::atomic::AtomicBool,
}

impl Installed {
    pub(crate) fn new(mods: Mods) -> Self {
        // Each hook's subscribers are ordered once here: by the priority their `#[listener]`
        // gives, then in dependency order.
        let subscriptions = mods.iter().enumerate().flat_map(|(index, entry)| {
            let listeners = entry.hooks.listeners();
            entry.hooks.subscribed().iter().flatten().map(move |hook| {
                let listener = listeners.iter().find(|listener| {
                    listener
                        .hook
                        .is_some_and(|id| id as usize == *hook as usize)
                });
                sunrise_mod_runtime::subscribers::Subscription {
                    index,
                    hook: *hook as usize,
                    priority: listener.map_or(0, |listener| listener.priority),
                    receives_cancelled: listener.is_some_and(|listener| listener.receive_cancelled),
                }
            })
        });
        let subscribers = sunrise_mod_runtime::subscribers::Subscribers::new(
            generated::HOOK_COUNT,
            mods.len(),
            subscriptions,
        );
        let disabled = mods
            .iter()
            .map(|_| std::sync::atomic::AtomicBool::new(false))
            .collect();
        let mod_id_owners = mods
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.id, index))
            .collect();
        Self {
            by_id: mods
                .iter()
                .enumerate()
                .map(|(i, entry)| (entry.id, i))
                .collect(),
            mods,
            subscribers,
            disabled,
            mod_id_owners,
            content_owners: OnceLock::new(),
            disabled_count: std::sync::atomic::AtomicUsize::new(0),
            poisons: std::sync::Mutex::new(None),
            poison_free: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

pub(crate) static MODS: OnceLock<Installed> = OnceLock::new();

#[cfg(any(test, feature = "testing"))]
thread_local! {
    pub(crate) static TEST_MODS: std::cell::Cell<Option<&'static Installed>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn installed() -> Option<&'static Installed> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(installed) = TEST_MODS.get() {
        return Some(installed);
    }
    MODS.get()
}

/// Install the process-wide ordered list of mods.
pub(crate) fn install(mods: Mods) -> &'static Installed {
    let installed = Installed::new(mods);
    #[cfg(any(test, feature = "testing"))]
    {
        // Tests can replace the list; mods() keeps its static slice lifetime.
        let installed = Box::leak(Box::new(installed));
        TEST_MODS.set(Some(installed));
        installed
    }
    #[cfg(not(any(test, feature = "testing")))]
    {
        assert!(MODS.set(installed).is_ok(), "trait mods are installed once");
        MODS.get().unwrap()
    }
}

/// Mark `id`'s listener of `hook` poisoned for the session (#2414 ruling):
/// it is skipped by that one hook alone — its other hooks and Parts are
/// untouched, unlike a whole-mod disable. Called only from a listener's
/// `catch_unwind` handler, so an installation is always in place.
pub(crate) fn poison(hook: generated::HookId, id: &'static str) {
    let installed = installed().expect("a poison lands inside a dispatch");
    let mut poisons = installed.poisons.lock().unwrap();
    let sets =
        poisons.get_or_insert_with(|| std::vec![None; generated::HOOK_COUNT].into_boxed_slice());
    sets[hook as usize]
        .get_or_insert_with(std::collections::HashSet::new)
        .insert(id);
    installed
        .poison_free
        .store(false, std::sync::atomic::Ordering::Release);
}

/// Whether `id`'s listener of `hook` is poisoned for the session.
pub(crate) fn poisoned(hook: generated::HookId, id: &str) -> bool {
    // One atomic load on the usual path: the sets only fill on a panic.
    installed().is_some_and(|installed| {
        !installed
            .poison_free
            .load(std::sync::atomic::Ordering::Acquire)
            && installed
                .poisons
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|poisons| poisons[hook as usize].as_ref())
                .is_some_and(|poisoned| poisoned.contains(id))
    })
}

/// The process-wide list in dispatch order, or an empty list before installation.
pub(crate) fn mods() -> &'static [ModEntry] {
    installed().map_or(&[], |installed| &installed.mods)
}

pub(crate) fn listened(hook: generated::HookId) -> bool {
    installed().is_some_and(|installed| installed.subscribers.contains_any(hook as usize))
}

pub(crate) fn enabled_listened(hook: generated::HookId) -> bool {
    installed().is_some_and(|installed| {
        installed
            .subscribers
            .indices(hook as usize)
            .any(|index| !installed.disabled[index].load(std::sync::atomic::Ordering::Relaxed))
    })
}

/// Switch an installed trait mod's subsequent dispatches on or off.
pub(crate) fn set_mod_enabled(id: &str, on: bool) -> bool {
    let Some(installed) = installed() else {
        return false;
    };
    let mut found = false;
    for (entry, disabled) in installed.mods.iter().zip(&*installed.disabled) {
        if entry.id == id {
            // Publish the slow path before disabling. A repeated disable reserves and releases
            // its own count; enabling releases the count of the previously disabled entry.
            // This keeps concurrent toggles from publishing a stale false flag.
            if !on {
                installed
                    .disabled_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            let was_disabled = disabled.swap(!on, std::sync::atomic::Ordering::SeqCst);
            if was_disabled {
                installed
                    .disabled_count
                    .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
            if was_disabled == on {
                MODS_SWITCHED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if !on {
                    sunrise_mod_runtime::water_tickets::remove_owner(id);
                }
            }
            found = true;
        }
    }
    found
}

/// Cached at load/toggle time, so the scheduler need not inspect content on its normal path.
pub(crate) fn any_mod_disabled() -> bool {
    installed().is_some_and(|installed| {
        installed
            .disabled_count
            .load(std::sync::atomic::Ordering::SeqCst)
            != 0
    })
}

/// How many times a mod was switched on or off: what a kept capability compares to tell that a
/// provider may have come or gone (`BlockCapabilityCache`).
static MODS_SWITCHED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The count of mod switches so far.
pub(crate) fn mods_switched() -> u64 {
    MODS_SWITCHED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether an installed mod subscribes to an injected hook.
// hidden: a game-side helper, not part of the mod surface
#[doc(hidden)]
pub fn subscribed_by(index: usize, hook: Option<generated::HookId>) -> bool {
    hook.is_some_and(|hook| {
        installed().is_some_and(|installed| {
            index < installed.mods.len()
                && !installed.disabled[index].load(std::sync::atomic::Ordering::Relaxed)
                && installed.subscribers.contains(hook as usize, index)
        })
    })
}

pub(crate) fn mod_index_on(index: usize) -> bool {
    installed().is_none_or(|installed| {
        installed
            .disabled
            .get(index)
            .is_none_or(|disabled| !disabled.load(std::sync::atomic::Ordering::Relaxed))
    })
}

/// Freeze primary IDs first, then the first admitted owner of each alias.
/// An admitted content-only owner reserves its aliases without a dispatch slot.
pub(crate) fn install_content_owners<'a>(owners: impl IntoIterator<Item = (&'a str, &'a str)>) {
    if let Some(installed) = installed() {
        installed.content_owners.get_or_init(|| {
            let mut by_namespace: std::collections::HashMap<Box<str>, _> = installed
                .mod_id_owners
                .iter()
                .map(|(&id, &index)| (id.into(), Some(index)))
                .collect();
            for (namespace, owner) in owners {
                by_namespace
                    .entry(namespace.into())
                    .or_insert_with(|| installed.mod_id_owners.get(owner).copied());
            }
            by_namespace
        });
    }
}

pub(crate) fn content_owners_published() -> bool {
    installed().is_none_or(|installed| installed.content_owners.get().is_some())
}

/// Content ownership is resolved at startup, not by scanning mods on a tick.
pub(crate) fn content_owner_disabled(id: &str) -> bool {
    any_mod_disabled() && content_owner_index(id).is_some_and(|index| !mod_index_on(index))
}

pub(crate) fn content_owner_index(id: &str) -> Option<usize> {
    let (namespace, _) = id.split_once(':')?;
    let installed = installed()?;
    match installed.content_owners.get() {
        Some(owners) => owners.get(namespace).copied().flatten(),
        None => installed.mod_id_owners.get(namespace).copied(),
    }
}

/// `mods` in order, each marked as running `hook` until the next one is (or the loop ends), so a
/// panic in it names the mod. The generated dispatch loops over it.
pub fn entered_registration<'a>(
    mods: &'a [ModEntry],
    hook: &'static str,
) -> impl Iterator<Item = &'a dyn Hooks> + 'a {
    Entered {
        mods: mods.iter(),
        hook,
        guard: None,
        scope: None,
    }
}

pub(crate) fn entered(
    hook_id: generated::HookId,
    hook: &'static str,
) -> impl Iterator<Item = &'static dyn Hooks> {
    entered_subscribers(hook_id, hook).map(|entry| &*entry.hooks)
}

/// [`entered`] with each mod's own `[mod] id`, for a hook that names the mod that answered.
pub(crate) fn entered_with_id(
    hook_id: generated::HookId,
    hook: &'static str,
) -> impl Iterator<Item = (&'static str, &'static dyn Hooks)> {
    entered_subscribers(hook_id, hook).map(|entry| (entry.id, &*entry.hooks))
}

/// [`entered`] for the listeners the game chose (`Listeners`), for a hook declared with
/// `select`: the others are passed over in the same dispatch order.
pub(crate) fn entered_selected<'a>(
    hook_id: generated::HookId,
    hook: &'static str,
    listeners: Listeners<'a>,
) -> impl Iterator<Item = &'static dyn Hooks> + 'a {
    entered_in(hook_id, hook, listeners).map(|entry| &*entry.hooks)
}

/// A hook's enabled subscribers in dispatch order (descending priority, then dependency order),
/// each marked as running `hook` until the next one is (or the loop ends), so a panic in it
/// names the mod.
pub(crate) fn entered_subscribers(
    hook_id: generated::HookId,
    hook: &'static str,
) -> impl Iterator<Item = &'static ModEntry> {
    entered_in(hook_id, hook, Listeners::All)
}

/// [`entered_subscribers`] for the listeners `listeners` takes.
fn entered_in<'a>(
    hook_id: generated::HookId,
    hook: &'static str,
    listeners: Listeners<'a>,
) -> impl Iterator<Item = &'static ModEntry> + 'a {
    let mut crash_guard = crate::dispatch_scope::ModGuard::empty();
    let scope = crate::dispatch_scope::snapshot();
    listening_in(hook_id, listeners).map(move |(_, entry)| {
        scope.replace(
            &mut crash_guard,
            crate::content::crash::ModCall {
                id: entry.id,
                hook,
                part: None,
            },
        );
        entry
    })
}

pub(crate) fn mark_mod(id: &'static str, hook: &'static str) -> crate::content::crash::ModGuard {
    crate::content::crash::enter_native_mod(crate::content::crash::ModCall {
        id,
        hook,
        part: None,
    })
}

/// The parts in `parts`, each marked as running `hook` as `entered` marks a mod, with its
/// type, so a panic in it names the mod, the hook and the part.
pub(crate) fn entered_parts<'a, P: ?Sized>(
    parts: &'a [Attached<P>],
    hook: &'static str,
) -> impl Iterator<Item = &'a P> {
    let mut guard = crate::dispatch_scope::ModGuard::empty();
    let scope = crate::dispatch_scope::snapshot();
    parts
        .iter()
        .filter(|attached| mod_index_on(attached.mod_index))
        .map(move |attached| {
            // The last part's mark goes first, as in `Entered`.
            scope.replace(
                &mut guard,
                crate::content::crash::ModCall {
                    id: attached.mod_id,
                    hook,
                    part: Some(attached.type_name),
                },
            );
            &*attached.part
        })
}

pub(crate) struct Entered<'a> {
    pub(crate) mods: std::slice::Iter<'a, ModEntry>,
    pub(crate) hook: &'static str,
    pub(crate) guard: Option<crate::content::crash::ModGuard>,
    scope: Option<crate::dispatch_scope::Scope>,
}

impl<'a> Iterator for Entered<'a> {
    type Item = &'a dyn Hooks;

    fn next(&mut self) -> Option<Self::Item> {
        // The last mod's mark goes first, so the next one's puts back what was there before.
        drop(self.guard.take());
        let entry = self.mods.next()?;
        self.guard = Some(
            self.scope
                .get_or_insert_with(crate::dispatch_scope::snapshot)
                .enter(crate::content::crash::ModCall {
                    id: entry.id,
                    hook: self.hook,
                    part: None,
                }),
        );
        Some(&*entry.hooks)
    }
}

/// Whether a mod claims a cancellable hook.
///
/// A mod still told once an earlier one cancelled marks its method `#[listener(receive_cancelled)]`
/// and reads the event's `cancelled` field, which it may set back to `false`:
///
/// ```
/// use mod_api::{ClientHost, Flow, Hooks, RenderGuiPreEvent};
/// #[derive(Default)]
/// struct Gui;
/// #[mod_api::sunrise_mod]
/// impl Hooks for Gui {
///     #[listener(priority = -10, receive_cancelled)]
///     fn render_gui_pre(&self, _: &dyn ClientHost, event: &mut RenderGuiPreEvent) -> Flow {
///         event.cancelled = false;
///         Flow::Continue
///     }
/// }
/// ```
///
/// A hook no mod can cancel has no cancel to receive, so the option doesn't compile there:
///
/// ```compile_fail
/// use mod_api::{ClientHost, ClientTickPreEvent, Hooks};
/// #[derive(Default)]
/// struct Ticker;
/// #[mod_api::sunrise_mod]
/// impl Hooks for Ticker {
///     #[listener(receive_cancelled)]
///     fn client_tick_pre(&self, _: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {}
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Flow {
    /// Go on to the next listener and then the game's own behaviour.
    Continue,
    /// Stop here; the game's own behaviour does not run.
    Cancel,
}

/// Whether a subscriber of a cancel hook is still told once the event is cancelled: the generated
/// dispatch of an event the game hands over cancelled returns early without one.
pub(crate) fn receives_cancelled(hook: generated::HookId) -> bool {
    installed().is_some_and(|installed| installed.subscribers.receives_cancelled(hook as usize))
}

/// A cancel hook's enabled subscribers in dispatch order, each with whether it is still told
/// once the event is cancelled. Unlike [`entered`], nothing is marked: the dispatch enters only
/// the mods it calls, through [`ListenerScope::enter`].
pub(crate) fn listening(
    hook_id: generated::HookId,
) -> impl Iterator<Item = (bool, &'static ModEntry)> {
    listening_in(hook_id, Listeners::All)
}

/// [`listening`] for the listeners `listeners` takes.
fn listening_in<'a>(
    hook_id: generated::HookId,
    listeners: Listeners<'a>,
) -> impl Iterator<Item = (bool, &'static ModEntry)> + 'a {
    installed().into_iter().flat_map(move |installed| {
        installed
            .subscribers
            .subscribers(hook_id as usize)
            .filter(move |subscriber| {
                listeners.takes(subscriber.index)
                    && !installed.disabled[subscriber.index]
                        .load(std::sync::atomic::Ordering::Relaxed)
            })
            .map(move |subscriber| {
                // Every listener of a hook the loop reaches is one more entered: scoped
                // resources end the previous listener's lease on it.
                bump_listener_serial();
                (
                    subscriber.receives_cancelled,
                    &installed.mods[subscriber.index],
                )
            })
    })
}

/// Look up the enabled subscriber owning a namespaced kind without walking the hook's listeners.
pub(crate) fn listening_kind_owner(hook: HookId, kind: &str) -> Option<&'static ModEntry> {
    let (namespace, _) = kind.split_once(':')?;
    let installed = installed()?;
    let &index = installed.by_id.get(namespace)?;
    if !installed.subscribers.contains(hook as usize, index)
        || installed.disabled[index].load(std::sync::atomic::Ordering::Relaxed)
    {
        return None;
    }
    bump_listener_serial();
    Some(&installed.mods[index])
}

/// Marks the thread as inside `entry`'s `hook` until the guard drops, and hands back its hooks.
/// One reusable instrumentation slot for loops that filter cancellation or ownership.
pub(crate) struct ListenerScope {
    scope: crate::dispatch_scope::Scope,
    guard: crate::dispatch_scope::ModGuard,
}
impl ListenerScope {
    pub(crate) fn new() -> Self {
        Self {
            scope: crate::dispatch_scope::snapshot(),
            guard: crate::dispatch_scope::ModGuard::empty(),
        }
    }
    #[inline]
    pub(crate) fn enter(
        &mut self,
        entry: &'static ModEntry,
        hook: &'static str,
    ) -> &'static dyn Hooks {
        self.scope.replace(
            &mut self.guard,
            crate::content::crash::ModCall {
                id: entry.id,
                hook,
                part: None,
            },
        );
        &*entry.hooks
    }
}

thread_local! {
    /// Every listener entered on this thread, including listeners of different nested hooks.
    static LISTENER_SERIAL: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[inline]
fn bump_listener_serial() {
    LISTENER_SERIAL.with(|serial| serial.set(serial.get().wrapping_add(1)));
}

pub(crate) fn listener_serial() -> u64 {
    LISTENER_SERIAL.with(std::cell::Cell::get)
}

/// Which of a hook's listeners one call goes to, chosen by the game for a hook declared with
/// `select` (the entity tick hooks: a mod that named a type hears its entities alone, the others
/// with their family). Never seen by a mod.
// hidden: the engine's own dispatch argument.
// no wire: it stays in the engine's call; the event a mod gets is the same.
// exhaustive: only the game builds or matches it, in the same change as a new choice.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub enum Listeners<'a> {
    /// Every listener.
    All,
    /// The listeners in the set.
    Only(&'a ModSet),
    /// The listeners not in the set.
    Except(&'a ModSet),
    /// The one listener with this index in the installed mods.
    One(usize),
}

impl Listeners<'_> {
    /// Whether the mod at `index` among the installed mods is one of them.
    fn takes(self, index: usize) -> bool {
        match self {
            Self::All => true,
            Self::Only(set) => set.contains(index),
            Self::Except(set) => !set.contains(index),
            Self::One(one) => one == index,
        }
    }
}

/// Some of the installed mods, by their index in the installed list, resolved at load.
// hidden: the engine's own selection of listeners.
// no wire: resolved on the host from what the mods registered; never sent to a mod.
// exhaustive: its fields are private, so mods cannot build or destructure it
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModSet {
    words: Vec<u64>,
}

impl ModSet {
    /// The set of the mods at `indices`.
    pub fn of(indices: impl IntoIterator<Item = usize>) -> Self {
        let mut set = Self::default();
        for index in indices {
            set.insert(index);
        }
        set
    }
    /// Adds the mod at `index`.
    pub fn insert(&mut self, index: usize) {
        let word = index / 64;
        if self.words.len() <= word {
            self.words.resize(word + 1, 0);
        }
        self.words[word] |= 1 << (index % 64);
    }
    /// Adds every mod of `other`.
    pub fn extend(&mut self, other: &Self) {
        if self.words.len() < other.words.len() {
            self.words.resize(other.words.len(), 0);
        }
        for (word, other) in self.words.iter_mut().zip(&other.words) {
            *word |= other;
        }
    }
    /// Whether the mod at `index` is in the set.
    pub fn contains(&self, index: usize) -> bool {
        self.words
            .get(index / 64)
            .is_some_and(|word| word & (1 << (index % 64)) != 0)
    }
    /// Whether the set holds no mod.
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|&word| word == 0)
    }
    /// The indices of its mods, in order.
    pub fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(w, &word)| {
            (0..64)
                .filter(move |bit| word & (1 << bit) != 0)
                .map(move |bit| w * 64 + bit)
        })
    }
}

#[cfg(test)]
mod listener_lease_tests {
    use super::*;
    use sunrise_mod_runtime::pending_children::{Decision, Error, PendingChildren};

    #[test]
    fn entering_a_different_hook_expires_a_retained_pending_child() {
        struct Listener;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Listener {
            fn mob_split(
                &self,
                _: &mut dyn crate::ServerHost,
                _: &mut crate::MobSplitEvent<'_>,
            ) -> Flow {
                Flow::Continue
            }
            fn baby_entity_spawn(
                &self,
                _: &mut dyn crate::ServerHost,
                _: &mut crate::BabyEntitySpawnEvent<'_>,
            ) -> Flow {
                Flow::Continue
            }
        }
        install(vec![ModEntry::new("lease-test", Box::new(Listener))].into());
        let mut arena = PendingChildren::default();
        arena
            .run(
                1,
                1,
                vec![7u32],
                |arena, action| {
                    let _outer = listening(generated::HookId::MobSplit).next().unwrap();
                    arena.sync_listener(listener_serial()).unwrap();
                    let child = arena.borrow(action, 0).unwrap();
                    assert_eq!(arena.get(child), Ok(&7));
                    let _nested = listening(generated::HookId::BabyEntitySpawn)
                        .next()
                        .unwrap();
                    arena.sync_listener(listener_serial()).unwrap();
                    assert_eq!(arena.get(child), Err(Error::Expired));
                    let fresh = arena.borrow(action, 0).unwrap();
                    assert_eq!(arena.get(fresh), Ok(&7));
                    Decision::Accept
                },
                |_| true,
            )
            .unwrap();
    }
}

#[cfg(test)]
mod toggle_tests {
    use super::*;

    #[test]
    fn cached_disabled_flag_tracks_repeated_and_independent_toggles() {
        struct Quiet;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Quiet {}
        crate::testing::with_dispatch(
            vec![
                ModEntry::new("a", Box::new(Quiet)),
                ModEntry::new("b", Box::new(Quiet)),
            ]
            .into(),
            || {
                assert!(!any_mod_disabled());
                assert!(set_mod_enabled("a", false));
                assert!(set_mod_enabled("a", false));
                assert!(any_mod_disabled());
                assert!(set_mod_enabled("b", false));
                assert!(set_mod_enabled("a", true));
                assert!(set_mod_enabled("a", true));
                assert!(any_mod_disabled());
                assert!(!set_mod_enabled("missing", true));
                assert!(any_mod_disabled());
                assert!(set_mod_enabled("b", true));
                assert!(!any_mod_disabled());
            },
        );
    }
}
