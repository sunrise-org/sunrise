//! Data-file load conditions: NeoForge's (`neoforge:conditions`) and Fabric API's
//! (`fabric:load_conditions`). A data file that has a key loads only when every condition
//! in it holds; a file with both keys loads only when both hold, since each loader reads only
//! its own key. A false condition silently omits the file; a NeoForge decode or handler error
//! goes to the owning loader, while a Fabric key that does not decode is logged and the file
//! loads, as Fabric's `ResourceConditionsImpl.applyResourceConditions` does.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// NeoForge's file conditions key (`ConditionalOps.DEFAULT_CONDITIONS_KEY`): a list.
pub const NEOFORGE_KEY: &str = "neoforge:conditions";
/// Fabric API's file conditions key (`ResourceConditions.CONDITIONS_KEY`): one condition, or a
/// list that must all hold.
pub const FABRIC_KEY: &str = "fabric:load_conditions";

/// Whether a data file carries either loader's top-level conditions key.
pub fn has_conditions(value: &Value) -> bool {
    value.get(NEOFORGE_KEY).is_some() || value.get(FABRIC_KEY).is_some()
}

/// The value passed to the selected registry codec by NeoForge's
/// `ConditionalOps.ConditionalDecoder`, after conditions have been evaluated. Fabric's key is no
/// schema field either (its codecs never read it), so it goes too.
pub fn into_payload(mut value: Value) -> Value {
    if let Some(fields) = value.as_object_mut() {
        fields.remove(FABRIC_KEY);
    }
    if let Some(fields) = value.as_object_mut()
        && fields.remove(NEOFORGE_KEY).is_some()
        && let Some(payload) = fields.remove("neoforge:value")
    {
        return payload;
    }
    value
}

/// Enabled package IDs captured by the engine for one owning data load.
/// Readers share this immutable view; they never recover IDs from namespaces.
/// Resource selection happens before evaluating ordinary JSON files. Stacked
/// data maps evaluate each layer before merge or replace; tags ignore conditions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LoadConditions {
    mods: Vec<String>,
    registered: Option<RegisteredLookup>,
    context: Context,
    diagnostics: Diagnostics,
}

impl LoadConditions {
    /// A vanilla data load has the built-in package only.
    pub const fn vanilla() -> Self {
        Self {
            mods: Vec::new(),
            registered: None,
            context: Context {
                features: None,
                feature_fingerprint: None,
                registries: None,
                tags: None,
                bound_tags: None,
            },
            diagnostics: Diagnostics::new(),
        }
    }

    /// Capture IDs visible to the owning load's side. The engine supplies its
    /// enabled-mod inventory's server view for logical-server data.
    pub fn new<'a>(ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut mods: Vec<_> = ids
            .into_iter()
            .filter(|id| *id != "minecraft")
            .map(str::to_owned)
            .collect();
        mods.sort();
        mods.dedup();
        Self {
            mods,
            registered: None,
            context: Context::default(),
            diagnostics: Diagnostics::default(),
        }
    }

    /// Use the recipe source's inventory while retaining this candidate's
    /// registered handlers, evaluation cache and diagnostics.
    pub fn with_mod_inventory<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut view = self.clone();
        view.mods = Self::new(ids).mods;
        if view.mods != self.mods {
            view.diagnostics = match &self.registered {
                Some(registered) => registered
                    .0
                    .inventories
                    .lock()
                    .unwrap()
                    .entry(view.mods.clone())
                    .or_default()
                    .clone(),
                // Without handlers there are no cached answers; keep registry
                // diagnostics shared across the candidate.
                None => self.diagnostics.clone(),
            };
        }
        view
    }

    pub fn mod_loaded(&self, id: &str) -> bool {
        id == "minecraft"
            || self
                .mods
                .binary_search_by(|loaded| loaded.as_str().cmp(id))
                .is_ok()
    }

    /// `IContext.enabledFeatures`: the world's enabled feature flag ids, which
    /// `neoforge:feature_flags_enabled` reads. Empty is vanilla's default set.
    pub fn with_features<'a>(mut self, flags: impl IntoIterator<Item = &'a str>) -> Self {
        let mut flags: BTreeSet<String> = flags.into_iter().map(qualified).collect();
        if flags.is_empty() {
            flags.insert("minecraft:vanilla".to_owned());
        }
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        flags.hash(&mut hasher);
        self.context.feature_fingerprint = Some(hasher.finish());
        self.context.features = Some(Arc::new(flags));
        self
    }

    /// `IContext.registries`: every registry's entry ids (built-in and the
    /// packs' datapack registries), which `neoforge:registered` reads.
    pub fn with_registries<'a, I>(
        mut self,
        registries: impl IntoIterator<Item = (&'a str, I)>,
    ) -> Self
    where
        I: IntoIterator<Item = &'a str>,
    {
        self.context.registries = Some(Arc::new(Names::new(registries)));
        self
    }

    /// The tags this load has bound: each registry's tags, each with whether it holds an
    /// entry. NeoForge's `neoforge:tag_empty` reads the ones holding an entry
    /// (`ICondition.IContext.getTag`); Fabric's `fabric:tags_populated` reads every bound tag,
    /// empty ones included (`HolderGetter.get(TagKey)` is present for a bound tag whatever it
    /// holds). Readers that run before the tags load never get this view, so a tag condition
    /// there omits the file instead of answering from an older load's tags.
    pub fn with_tags<'a, I>(&self, tags: impl IntoIterator<Item = (&'a str, I)>) -> Self
    where
        I: IntoIterator<Item = (&'a str, bool)>,
    {
        let tags: Vec<(&str, Vec<(&str, bool)>)> = tags
            .into_iter()
            .map(|(registry, tags)| (registry, tags.into_iter().collect()))
            .collect();
        let mut view = self.clone();
        view.context.tags = Some(Arc::new(Names::new(tags.iter().map(|(registry, tags)| {
            let held = tags.iter().filter(|(_, held)| *held);
            (*registry, held.map(|(tag, _)| *tag))
        }))));
        view.context.bound_tags =
            Some(Arc::new(Names::new(tags.iter().map(|(registry, tags)| {
                (*registry, tags.iter().map(|(tag, _)| *tag))
            }))));
        view
    }

    /// `ConditionalOps.decodeListWithElementConditions` on one list: drop an
    /// element whose own `neoforge:conditions` fail and strip the key from one
    /// whose conditions hold. Only the lists NeoForge decodes that way carry
    /// element conditions (a loot table's pools, data-map values, overlays and
    /// atlas sources, each read by its own reader); anywhere else a nested
    /// `neoforge:conditions` key is ordinary data and is never evaluated.
    fn filter_list(&self, elements: &mut Vec<Value>, path: &dyn Fn() -> String) -> bool {
        let mut valid = true;
        // Element conditions are NeoForge's alone: Fabric reads its key on whole files only.
        elements.retain_mut(
            |element| match self.registry_verdict(element, Keys::NeoForge, path) {
                Some(true) => {
                    if let Some(fields) = element.as_object_mut() {
                        fields.remove(NEOFORGE_KEY);
                    }
                    true
                }
                Some(false) => false,
                None => {
                    valid = false;
                    false
                }
            },
        );
        valid
    }

    /// `allows_id` for a loot table, then its `pools` filtered by their own
    /// conditions: NeoForge decodes a table's pool list with element conditions
    /// (a false pool is dropped, a passing one loses the key). A pool's
    /// `entries` and `functions` and an entry's `children` are plain lists.
    pub(crate) fn admit_loot_table(&self, file: &mut Value, id: &str) -> bool {
        if !self.allows_id(file, "loot_table", id) {
            return false;
        }
        let path = || {
            let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
            format!("data/{namespace}/loot_table/{path}.json")
        };
        file.get_mut("pools")
            .and_then(Value::as_array_mut)
            .is_none_or(|pools| self.filter_list(pools, &path))
    }

    /// Attach the engine's immutable registration table to this candidate load.
    pub fn with_lookup<S, F>(mut self, handlers: impl IntoIterator<Item = (S, F)>) -> Self
    where
        S: Into<String>,
        F: Fn(&Value) -> Result<bool, String> + Send + Sync + 'static,
    {
        let handlers: Handlers = handlers
            .into_iter()
            .map(|(id, handler)| (id.into(), Box::new(handler) as Box<ConditionHandler>))
            .collect();
        if handlers.is_empty() {
            return self;
        }
        if self.diagnostics.0.is_none() {
            self.diagnostics = Diagnostics::default();
        }
        self.registered = Some(RegisteredLookup(Arc::new(LookupState {
            handlers,
            inventories: std::sync::Mutex::new(BTreeMap::from([(
                self.mods.clone(),
                self.diagnostics.clone(),
            )])),
        })));
        self
    }

    /// Whether this candidate has any registered condition handlers.
    pub fn has_registered(&self) -> bool {
        self.registered.is_some()
    }

    pub fn allows(&self, file: &Value) -> bool {
        self.allows_file(file, "data file")
    }

    /// Evaluate a resource, constructing its path only for registered handlers or diagnostics.
    pub fn allows_id(&self, file: &Value, kind: &str, id: &str) -> bool {
        self.allows_context(file, || {
            let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
            format!("data/{namespace}/{kind}/{path}.json")
        })
    }

    /// Evaluate a resource whose namespace and path are stored separately.
    pub fn allows_parts(&self, file: &Value, kind: &str, namespace: &str, path: &str) -> bool {
        self.allows_context(file, || format!("data/{namespace}/{kind}/{path}.json"))
    }

    /// Evaluate a registry entry. Its owning load checks accumulated errors
    /// before it publishes the registry.
    pub fn allows_file(&self, file: &Value, path: &str) -> bool {
        self.allows_context(file, || path.to_owned())
    }

    /// Return a file's error without adding it to registry diagnostics. The
    /// caller decides whether to skip the file or refuse its own candidate.
    pub fn allows_file_isolated(&self, file: &Value, path: &str) -> Result<bool, String> {
        self.verdict(file, Keys::Both, &|| path.to_owned())
            .map_err(|error| error.message().to_owned())
    }

    fn allows_context(&self, file: &Value, path: impl Fn() -> String) -> bool {
        self.registry_verdict(file, Keys::Both, &path)
            .unwrap_or(false)
    }

    /// A false condition omits the entry; a decode or handler error also
    /// invalidates the enclosing registry file (for example a loot table).
    fn registry_verdict(
        &self,
        file: &Value,
        keys: Keys,
        path: &dyn Fn() -> String,
    ) -> Option<bool> {
        match self.verdict(file, keys, path) {
            Ok(answer) => Some(answer),
            Err(error) => {
                let diagnostic = format!("{}: {}", path(), error.message());
                // Some early readers do not have all of the built-in contexts.
                // Preserve their existing omission rather than reading stale tags.
                let invalid = matches!(error, ConditionError::Invalid(_));
                if let Some(state) = self.diagnostics.0.as_ref().filter(|_| invalid) {
                    state.lock().unwrap().errors.insert(diagnostic);
                } else {
                    log::error!("{diagnostic}; file left out");
                }
                if invalid { None } else { Some(false) }
            }
        }
    }

    fn verdict(&self, file: &Value, keys: Keys, path: &dyn Fn() -> String) -> Verdict {
        self.decode_file(file, keys, path)?
            .verdict(&self.context, path)
    }

    /// Decode the file's entire condition program (both keys) without testing it or adding to
    /// registry diagnostics. `path` names the file in the log line of a Fabric key that does
    /// not decode.
    pub fn decode_file_conditions<'a>(
        &'a self,
        file: &'a Value,
        path: &str,
    ) -> Result<DecodedFileConditions<'a>, String> {
        self.decode_file(file, Keys::Both, &|| path.to_owned())
            .map_err(|error| error.message().to_owned())
    }

    /// The file's condition program: NeoForge's list, then Fabric's condition, all of which must
    /// hold. A NeoForge decode error refuses the program. Fabric's key that does not decode is
    /// logged and left out, as `ResourceConditionsImpl.applyResourceConditions` loads the file.
    fn decode_file<'a>(
        &'a self,
        file: &'a Value,
        keys: Keys,
        path: &dyn Fn() -> String,
    ) -> Result<DecodedFileConditions<'a>, ConditionError> {
        let handlers = self.registered.as_ref().map(|lookup| &lookup.0.handlers);
        let payload = file.get(NEOFORGE_KEY);
        let mut nodes = match payload {
            Some(payload) => Condition::decode_list(payload, handlers)?,
            None => Vec::new(),
        };
        let fabric = match keys {
            Keys::Both => file.get(FABRIC_KEY),
            Keys::NeoForge => None,
        };
        let fabric =
            fabric.filter(
                |fabric| match Condition::decode_fabric_root(fabric, handlers) {
                    Ok(condition) => {
                        nodes.push(condition);
                        true
                    }
                    Err(error) => {
                        // Logged once per load candidate, as `applyResourceConditions` runs once per
                        // file. A load without a candidate (`LoadConditions::vanilla`) keeps no record
                        // and logs each evaluation.
                        let line = format!(
                            "{}: {FABRIC_KEY}: {error}; the file loads without its conditions",
                            path()
                        );
                        let first =
                            self.diagnostics.0.as_ref().is_none_or(|state| {
                                state.lock().unwrap().logged.insert(line.clone())
                            });
                        if first {
                            log::error!("{line}");
                        }
                        false
                    }
                },
            );
        Ok(DecodedFileConditions {
            load: self,
            payload,
            fabric,
            nodes,
        })
    }

    /// Refuse a failed candidate before publishing any of its tables.
    pub fn check_errors(&self) -> Result<(), String> {
        if let Some(state) = self.diagnostics.0.as_ref() {
            let state = state.lock().unwrap();
            if !state.errors.is_empty() {
                return Err(state.errors.iter().cloned().collect::<Vec<_>>().join("; "));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum ConditionError {
    Invalid(String),
    MissingContext(&'static str),
}
impl ConditionError {
    fn message(&self) -> &str {
        match self {
            Self::Invalid(message) => message,
            Self::MissingContext(message) => message,
        }
    }
}

type Verdict = Result<bool, ConditionError>;
type CachedVerdict = Arc<std::sync::OnceLock<Verdict>>;

#[derive(Debug, Default)]
struct CandidateDiagnostics {
    cache: BTreeMap<(String, String, u64), CachedVerdict>,
    errors: BTreeSet<String>,
    /// Fabric keys that did not decode and were logged, once per candidate load.
    logged: BTreeSet<String>,
}
// Diagnostics do not contribute to a resource generation's cache identity.
#[derive(Clone, Debug)]
struct Diagnostics(Option<std::sync::Arc<std::sync::Mutex<CandidateDiagnostics>>>);
impl Diagnostics {
    const fn new() -> Self {
        Self(None)
    }
}
impl Default for Diagnostics {
    fn default() -> Self {
        Self(Some(Default::default()))
    }
}
impl PartialEq for Diagnostics {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Diagnostics {}
impl std::hash::Hash for Diagnostics {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

/// `ICondition.IContext`: what `feature_flags_enabled`, `registered` and
/// `tag_empty` read. A part the owning load didn't provide leaves its
/// condition unsupported: the file is omitted with one error, never answered stale.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct Context {
    features: Option<Arc<BTreeSet<String>>>,
    feature_fingerprint: Option<u64>,
    registries: Option<Arc<Names>>,
    /// The bound tags holding an entry.
    tags: Option<Arc<Names>>,
    /// Every bound tag, empty ones included.
    bound_tags: Option<Arc<Names>>,
}
impl Context {
    /// The cache key of answers given under this context.
    fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        (
            &self.feature_fingerprint,
            &self.registries,
            &self.tags,
            &self.bound_tags,
        )
            .hash(&mut hasher);
        hasher.finish()
    }
}

/// Registry id -> entry ids, compared and hashed by a fingerprint taken once,
/// so cache identity never rehashes every entry.
#[derive(Debug)]
struct Names {
    by_registry: BTreeMap<String, BTreeSet<String>>,
    fingerprint: u64,
}
impl Names {
    fn new<'a, I>(registries: impl IntoIterator<Item = (&'a str, I)>) -> Self
    where
        I: IntoIterator<Item = &'a str>,
    {
        use std::hash::{Hash, Hasher};
        let mut by_registry: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (registry, ids) in registries {
            by_registry
                .entry(qualified(registry))
                .or_default()
                .extend(ids.into_iter().map(qualified));
        }
        let mut hasher = std::hash::DefaultHasher::new();
        by_registry.hash(&mut hasher);
        Self {
            by_registry,
            fingerprint: hasher.finish(),
        }
    }
    fn contains(&self, registry: &str, id: &str) -> bool {
        self.by_registry
            .get(registry)
            .is_some_and(|ids| ids.contains(id))
    }
}
impl PartialEq for Names {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
}
impl Eq for Names {}
impl std::hash::Hash for Names {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.fingerprint.hash(state);
    }
}

/// An identifier with vanilla's default namespace (`Identifier.parse`).
fn qualified(id: &str) -> String {
    let id = id.strip_prefix('#').unwrap_or(id);
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

// Pass the raw node without copying; the registration adapter removes `type`
// before passing the remaining fields to the mod.
type ConditionHandler = dyn Fn(&Value) -> Result<bool, String> + Send + Sync;
type Handlers = BTreeMap<String, Box<ConditionHandler>>;
struct LookupState {
    inventories: std::sync::Mutex<BTreeMap<Vec<String>, Diagnostics>>,
    handlers: Handlers,
}
#[derive(Clone)]
struct RegisteredLookup(std::sync::Arc<LookupState>);
impl std::fmt::Debug for RegisteredLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegisteredLookup")
    }
}
impl PartialEq for RegisteredLookup {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for RegisteredLookup {}
impl std::hash::Hash for RegisteredLookup {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&std::sync::Arc::as_ptr(&self.0), state);
    }
}

/// Which conditions keys an evaluation reads: a whole file reads both loaders', an element of a
/// list NeoForge decodes with element conditions (a loot pool) only NeoForge's.
#[derive(Clone, Copy)]
enum Keys {
    Both,
    NeoForge,
}

/// A file's decoded condition program, borrowing its payloads and the load that knows its
/// handlers. `fabric` is set only when the Fabric key decoded into the program.
pub struct DecodedFileConditions<'a> {
    load: &'a LoadConditions,
    payload: Option<&'a Value>,
    fabric: Option<&'a Value>,
    nodes: Vec<Condition<'a>>,
}

/// The outcome of advancing a condition program in a staged registry load.
pub enum ConditionProgress<'a> {
    Complete(bool),
    NeedsRegistry(&'a str),
}

/// A suspended program keeps its place in the condition tree, not a cached verdict.
/// In particular, reaching a pending registry never re-invokes an earlier callback.
pub struct ConditionEvaluation<'program, 'load> {
    load: &'load LoadConditions,
    frames: Vec<EvaluationFrame<'program, 'load>>,
    missing_context: Option<&'static str>,
}

enum EvaluationFrame<'program, 'load> {
    List(bool, std::slice::Iter<'program, Condition<'load>>),
    Not,
    Operand(&'program Condition<'load>),
}

impl<'program, 'load> ConditionEvaluation<'program, 'load> {
    /// Test reached operands against the current view, stopping before a pending registry read.
    pub fn advance(
        &mut self,
        view: &LoadConditions,
        pending: impl Fn(&str) -> bool,
    ) -> Result<ConditionProgress<'program>, String> {
        self.advance_context(&view.context, pending)
            .map_err(|error| error.message().to_owned())
    }

    fn advance_context(
        &mut self,
        context: &Context,
        pending: impl Fn(&str) -> bool,
    ) -> Result<ConditionProgress<'program>, ConditionError> {
        loop {
            let mut result = match self.frames.last_mut() {
                Some(EvaluationFrame::List(and, children)) => match children.next() {
                    Some(node) => {
                        self.frames.push(EvaluationFrame::Operand(node));
                        continue;
                    }
                    None => *and,
                },
                Some(EvaluationFrame::Operand(node)) => match *node {
                    Condition::Combinator { and, children } => {
                        let frame = EvaluationFrame::List(*and, children.iter());
                        self.frames.pop();
                        self.frames.push(frame);
                        continue;
                    }
                    Condition::Not(child) => {
                        let frame = EvaluationFrame::Operand(child);
                        self.frames.pop();
                        self.frames.push(EvaluationFrame::Not);
                        self.frames.push(frame);
                        continue;
                    }
                    Condition::Registered { registry, .. }
                    | Condition::TagEmpty { registry, .. }
                    | Condition::TagBound { registry, .. }
                        if pending(registry) =>
                    {
                        return Ok(ConditionProgress::NeedsRegistry(registry));
                    }
                    node => node.test_operand(self.load, context, &mut self.missing_context)?,
                },
                _ => unreachable!("a running condition has a list or operand"),
            };
            self.frames.pop();
            // A list continues only so long as its result is undecided. Unwind a
            // decided result through NOT and the parent lists without reading their tails.
            loop {
                match self.frames.last() {
                    Some(EvaluationFrame::List(and, _)) if *and == result => break,
                    Some(EvaluationFrame::Not) => result = !result,
                    Some(EvaluationFrame::List(..)) => {}
                    _ => {
                        return self
                            .missing_context
                            .map_or(Ok(ConditionProgress::Complete(result)), |reason| {
                                Err(ConditionError::MissingContext(reason))
                            });
                    }
                }
                self.frames.pop();
            }
        }
    }
}

impl<'load> DecodedFileConditions<'load> {
    /// Start testing an already decoded program in candidate dependency order.
    pub fn evaluation(&self) -> ConditionEvaluation<'_, 'load> {
        ConditionEvaluation {
            load: self.load,
            frames: vec![EvaluationFrame::List(true, self.nodes.iter())],
            missing_context: None,
        }
    }

    fn verdict(&self, context: &Context, path: &dyn Fn() -> String) -> Verdict {
        if self.payload.is_none() && self.fabric.is_none() {
            return Ok(true);
        }
        let evaluate = || match self.evaluation().advance_context(context, |_| false)? {
            ConditionProgress::Complete(allowed) => Ok(allowed),
            ConditionProgress::NeedsRegistry(_) => unreachable!("no pending registries"),
        };
        if !self.load.has_registered() {
            return evaluate();
        }
        let path = path();
        let path = if path.starts_with("data/") {
            path.as_str()
        } else {
            path.rfind("/data/")
                .map_or(path.as_str(), |i| &path[i + 1..])
        };
        let state = self
            .load
            .diagnostics
            .0
            .as_ref()
            .expect("owning candidate diagnostics");
        // Context-free answers are shared across candidate views. Cache the
        // result, not its loader's recovery policy.
        let reads_context = self.nodes.iter().any(Condition::reads_context);
        let cell = state
            .lock()
            .unwrap()
            .cache
            .entry((
                path.to_owned(),
                // Both payloads, so a file's answer is cached per the pair it carries.
                format!(
                    "{}\n{}",
                    self.payload.map(Value::to_string).unwrap_or_default(),
                    self.fabric.map(Value::to_string).unwrap_or_default()
                ),
                if reads_context { context.key() } else { 0 },
            ))
            .or_default()
            .clone();
        cell.get_or_init(evaluate).clone()
    }
}

/// Fabric API's built-in condition types (`DefaultResourceConditionTypes`), which
/// `Condition::decode_fabric` maps onto the evaluator's own nodes.
const FABRIC_BUILTINS: [&str; 10] = [
    "true",
    "false",
    "not",
    "and",
    "or",
    "all_mods_loaded",
    "any_mods_loaded",
    "tags_populated",
    "features_enabled",
    "registry_contains",
];

/// Does the built-in evaluator already implement this condition ID (NeoForge's or Fabric's)?
pub fn is_builtin_condition(id: &sunrise_core::identifier::Identifier) -> bool {
    BuiltinCondition::from_parts(id.namespace(), id.path()).is_some()
        || (id.namespace() == "fabric" && FABRIC_BUILTINS.contains(&id.path()))
}

/// `FeatureFlags.REGISTRY`'s flags, which a feature condition may name.
const KNOWN_FEATURE_FLAGS: [&str; 4] = [
    "minecraft:vanilla",
    "minecraft:trade_rebalance",
    "minecraft:redstone_experiments",
    "minecraft:minecart_improvements",
];

#[derive(Clone, Copy)]
enum BuiltinCondition {
    ModLoaded,
    Always,
    Never,
    FeatureFlags,
    Registered,
    TagEmpty,
    Not,
    And,
    Or,
}

impl BuiltinCondition {
    fn from_parts(namespace: &str, path: &str) -> Option<Self> {
        if namespace != "neoforge" {
            return None;
        }
        Some(match path {
            "mod_loaded" => Self::ModLoaded,
            "always" => Self::Always,
            "never" => Self::Never,
            "feature_flags_enabled" => Self::FeatureFlags,
            "registered" => Self::Registered,
            "tag_empty" => Self::TagEmpty,
            "not" => Self::Not,
            "and" => Self::And,
            "or" => Self::Or,
            _ => return None,
        })
    }
}

enum Condition<'a> {
    ModLoaded(&'a str),
    Literal(bool),
    FeatureFlags(Vec<String>),
    Registered {
        registry: String,
        value: String,
    },
    TagEmpty {
        registry: String,
        tag: String,
    },
    /// Whether the load bound the tag at all, whatever it holds (Fabric's `tags_populated`).
    TagBound {
        registry: String,
        tag: String,
    },
    Not(Box<Self>),
    Combinator {
        and: bool,
        children: Vec<Self>,
    },
    /// A registered handler, the condition's node and the key that named it (`type` for
    /// NeoForge, `condition` for Fabric), which the handler's arguments leave out.
    Custom(&'a ConditionHandler, &'a Value, &'static str),
}

#[derive(Default)]
struct DecodeErrors<'a> {
    unknown: BTreeSet<&'a str>,
    invalid: BTreeSet<String>,
}

impl<'a> Condition<'a> {
    fn decode_list(
        payload: &'a Value,
        handlers: Option<&'a Handlers>,
    ) -> Result<Vec<Self>, ConditionError> {
        let nodes = payload.as_array().ok_or_else(|| {
            ConditionError::Invalid("neoforge:conditions: expected a list".into())
        })?;
        // ICondition.LIST_CODEC decodes even branches that cannot affect the boolean answer.
        let mut errors = DecodeErrors::default();
        let conditions = nodes
            .iter()
            .filter_map(|node| Self::decode(node, handlers, &mut errors))
            .collect();
        if !errors.unknown.is_empty() {
            errors.invalid.insert(format!(
                "unknown load conditions [{}]",
                errors.unknown.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        if !errors.invalid.is_empty() {
            return Err(ConditionError::Invalid(
                errors.invalid.into_iter().collect::<Vec<_>>().join("; "),
            ));
        }
        Ok(conditions)
    }

    fn decode(
        node: &'a Value,
        handlers: Option<&'a Handlers>,
        errors: &mut DecodeErrors<'a>,
    ) -> Option<Self> {
        let Some(name) = node.get("type").and_then(Value::as_str) else {
            errors
                .invalid
                .insert("load condition: missing or invalid type".into());
            return None;
        };
        if let Some(kind) = name
            .split_once(':')
            .and_then(|(ns, path)| BuiltinCondition::from_parts(ns, path))
        {
            return match Self::decode_builtin(kind, node, handlers, errors) {
                Ok(condition) => condition,
                Err(field) => {
                    errors.invalid.insert(format!("{name}: invalid {field}"));
                    None
                }
            };
        }
        let handler = handlers.and_then(|handlers| match name.split_once(':') {
            Some((ns, _)) if !ns.is_empty() => handlers.get(name),
            _ => handlers.get(&format!(
                "minecraft:{}",
                name.strip_prefix(':').unwrap_or(name)
            )),
        });
        if let Some(handler) = handler {
            Some(Self::Custom(handler.as_ref(), node, "type"))
        } else {
            errors.unknown.insert(name);
            None
        }
    }

    fn decode_builtin(
        kind: BuiltinCondition,
        node: &'a Value,
        handlers: Option<&'a Handlers>,
        errors: &mut DecodeErrors<'a>,
    ) -> Result<Option<Self>, &'static str> {
        let identifier = |value: &Value, field| {
            value
                .as_str()
                .and_then(|id| sunrise_core::identifier::Identifier::parse(id).ok())
                .map(|id| id.to_string())
                .ok_or(field)
        };
        let registry = || {
            node.get("registry").map_or_else(
                || Ok("minecraft:item".into()),
                |value| identifier(value, "registry"),
            )
        };
        use BuiltinCondition::*;
        Ok(Some(match kind {
            Always => Self::Literal(true),
            Never => Self::Literal(false),
            ModLoaded => Self::ModLoaded(node["modid"].as_str().ok_or("modid")?),
            FeatureFlags => Self::FeatureFlags(
                node["flags"]
                    .as_array()
                    .filter(|flags| !flags.is_empty())
                    .ok_or("flags")?
                    .iter()
                    .map(|value| {
                        let id = identifier(value, "flags")?;
                        // FeatureFlags.REGISTRY, not the enabled set in the condition context.
                        match id.as_str() {
                            known if KNOWN_FEATURE_FLAGS.contains(&known) => Ok(id),
                            _ => {
                                errors.invalid.insert(format!(
                                    "neoforge:feature_flags_enabled: unknown flag {id}"
                                ));
                                Err("flags")
                            }
                        }
                    })
                    .collect::<Result<_, _>>()?,
            ),
            Registered => Self::Registered {
                registry: registry()?,
                value: identifier(&node["value"], "value")?,
            },
            TagEmpty => Self::TagEmpty {
                registry: registry()?,
                tag: identifier(&node["tag"], "tag")?,
            },
            Not => {
                let child = node.get("value").ok_or("value")?;
                return Ok(
                    Self::decode(child, handlers, errors).map(|child| Self::Not(Box::new(child)))
                );
            }
            kind @ (And | Or) => Self::Combinator {
                and: matches!(kind, And),
                children: node["values"]
                    .as_array()
                    .ok_or("values")?
                    .iter()
                    .filter_map(|child| Self::decode(child, handlers, errors))
                    .collect(),
            },
        }))
    }

    /// Fabric API's `ResourceCondition.CONDITION_CODEC`: one condition, or a list of them that
    /// must all hold (`ResourceConditions.and`).
    fn decode_fabric_root(
        payload: &'a Value,
        handlers: Option<&'a Handlers>,
    ) -> Result<Self, String> {
        match payload {
            Value::Array(list) => Ok(Self::Combinator {
                and: true,
                children: list
                    .iter()
                    .map(|node| Self::decode_fabric(node, handlers))
                    .collect::<Result<_, _>>()?,
            }),
            node => Self::decode_fabric(node, handlers),
        }
    }

    /// One Fabric condition, dispatched on its `condition` id
    /// (`ResourceCondition.CODEC`). The built-in types are fabric-resource-conditions-api-v1's
    /// `DefaultResourceConditionTypes`, each mapped onto the evaluator's own nodes; any other id
    /// goes to the handler registered under it, as a NeoForge `type` does.
    fn decode_fabric(node: &'a Value, handlers: Option<&'a Handlers>) -> Result<Self, String> {
        let name = node
            .get("condition")
            .and_then(Value::as_str)
            .ok_or("load condition: missing or invalid condition")?;
        let id = |value: &Value, field: &str| {
            value
                .as_str()
                .and_then(|id| sunrise_core::identifier::Identifier::parse(id).ok())
                .map(|id| id.to_string())
                .ok_or_else(|| format!("{name}: invalid {field}"))
        };
        let list = |field: &str| {
            node.get(field)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{name}: invalid {field}"))
        };
        // `fieldOf("registry").orElse(Registries.ITEM)`: a missing or invalid id is the item
        // registry (DFU's `orElse` answers its value for any field error).
        let registry = || -> Result<String, String> {
            Ok(node
                .get("registry")
                .and_then(|value| id(value, "registry").ok())
                .unwrap_or_else(|| "minecraft:item".to_owned()))
        };
        let qualified = sunrise_core::identifier::Identifier::parse(name)
            .map_err(|_| format!("{name}: invalid condition id"))?
            .to_string();
        Ok(match qualified.as_str() {
            "fabric:true" => Self::Literal(true),
            "fabric:false" => Self::Literal(false),
            "fabric:not" => Self::Not(Box::new(Self::decode_fabric(
                node.get("value")
                    .ok_or_else(|| format!("{name}: invalid value"))?,
                handlers,
            )?)),
            kind @ ("fabric:and" | "fabric:or") => Self::Combinator {
                and: kind == "fabric:and",
                children: list("values")?
                    .iter()
                    .map(|child| Self::decode_fabric(child, handlers))
                    .collect::<Result<_, _>>()?,
            },
            // `ResourceConditionsImpl.modsLoaded`: the ids are plain strings.
            kind @ ("fabric:all_mods_loaded" | "fabric:any_mods_loaded") => Self::Combinator {
                and: kind == "fabric:all_mods_loaded",
                children: list("values")?
                    .iter()
                    .map(|id| {
                        id.as_str()
                            .map(Self::ModLoaded)
                            .ok_or_else(|| format!("{name}: invalid values"))
                    })
                    .collect::<Result<_, _>>()?,
            },
            // `ResourceConditionsImpl.tagsPopulated`: every tag is bound (`HolderGetter.get`
            // is present), even one that holds nothing.
            "fabric:tags_populated" => {
                let registry = registry()?;
                Self::Combinator {
                    and: true,
                    children: list("values")?
                        .iter()
                        .map(|tag| {
                            Ok(Self::TagBound {
                                registry: registry.clone(),
                                tag: id(tag, "values")?,
                            })
                        })
                        .collect::<Result<_, String>>()?,
                }
            }
            // `ResourceConditionsImpl.featuresEnabled`: an unknown flag is a failed test, not a
            // decode error.
            "fabric:features_enabled" => {
                let flags = list("features")?
                    .iter()
                    .map(|flag| id(flag, "features"))
                    .collect::<Result<Vec<_>, _>>()?;
                if flags
                    .iter()
                    .all(|flag| KNOWN_FEATURE_FLAGS.contains(&flag.as_str()))
                {
                    Self::FeatureFlags(flags)
                } else {
                    Self::Literal(false)
                }
            }
            // `ResourceConditionsImpl.registryContains`: every entry is registered.
            "fabric:registry_contains" => {
                let registry = registry()?;
                Self::Combinator {
                    and: true,
                    children: list("values")?
                        .iter()
                        .map(|entry| {
                            Ok(Self::Registered {
                                registry: registry.clone(),
                                value: id(entry, "values")?,
                            })
                        })
                        .collect::<Result<_, String>>()?,
                }
            }
            _ => handlers
                .and_then(|handlers| handlers.get(&qualified))
                .map(|handler| Self::Custom(handler.as_ref(), node, "condition"))
                .ok_or_else(|| format!("unknown load condition {name}"))?,
        })
    }

    fn reads_context(&self) -> bool {
        match self {
            Self::FeatureFlags(_)
            | Self::Registered { .. }
            | Self::TagEmpty { .. }
            | Self::TagBound { .. } => true,
            Self::Not(child) => child.reads_context(),
            Self::Combinator { children, .. } => children.iter().any(Self::reads_context),
            Self::ModLoaded(_) | Self::Literal(_) | Self::Custom(..) => false,
        }
    }

    fn test_operand(
        &self,
        load: &LoadConditions,
        context: &Context,
        missing_context: &mut Option<&'static str>,
    ) -> Verdict {
        let mut unavailable = |reason| {
            missing_context.get_or_insert(reason);
            Ok(false)
        };
        Ok(match self {
            Self::ModLoaded(id) => load.mod_loaded(id),
            Self::Literal(value) => *value,
            Self::FeatureFlags(flags) => {
                let Some(enabled) = &context.features else {
                    return unavailable(
                        "neoforge:feature_flags_enabled: no feature flags in this load",
                    );
                };
                flags.iter().all(|flag| enabled.contains(flag))
            }
            Self::Registered { registry, value } => {
                let Some(registries) = &context.registries else {
                    return unavailable("neoforge:registered: no registries in this load");
                };
                registries.contains(registry, value)
            }
            Self::TagEmpty { registry, tag } => {
                let Some(tags) = &context.tags else {
                    return unavailable("neoforge:tag_empty: tags not loaded yet");
                };
                !tags.contains(registry, tag)
            }
            Self::TagBound { registry, tag } => {
                let Some(tags) = &context.bound_tags else {
                    return unavailable("fabric:tags_populated: tags not loaded yet");
                };
                tags.contains(registry, tag)
            }
            Self::Not(_) | Self::Combinator { .. } => unreachable!("not an operand"),
            Self::Custom(handler, node, key) => {
                // The arguments are the node without the key that named the condition.
                let mut args = node.as_object().cloned().unwrap_or_default();
                args.remove(*key);
                handler(&Value::Object(args)).map_err(ConditionError::Invalid)?
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn load_conditions_capture_package_ids_and_keep_their_generation() {
        let mut enabled = vec!["code_only".to_owned()];
        let old = LoadConditions::new(enabled.iter().map(String::as_str));
        enabled.clear();
        let next = LoadConditions::new(enabled.iter().map(String::as_str));
        let requires_code = json!({"neoforge:conditions":[{
            "type":"neoforge:mod_loaded", "modid":"code_only"
        }]});
        assert!(old.allows(&requires_code));
        assert!(!next.allows(&requires_code));
        assert!(old.mod_loaded("minecraft") && next.mod_loaded("minecraft"));
        assert!(!old.mod_loaded("resource_namespace"));
        assert!(old.allows(&json!({"values":[]})));
        assert!(!old.allows(&json!({"neoforge:conditions":[{"type":"future:condition"}]})));
    }

    #[test]
    fn load_condition_cache_identity_uses_membership_not_input_order() {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let key = |conditions: &LoadConditions| {
            let mut hasher = DefaultHasher::new();
            conditions.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(
            key(&LoadConditions::new(["b", "a", "a"])),
            key(&LoadConditions::new(["a", "b"]))
        );
        assert_ne!(
            key(&LoadConditions::new(["a"])),
            key(&LoadConditions::vanilla())
        );
        assert_eq!(
            LoadConditions::new(["minecraft"]),
            LoadConditions::vanilla()
        );
    }

    #[test]
    fn a_file_without_conditions_loads() {
        let conditions = LoadConditions::vanilla();
        assert!(conditions.allows(&json!({"type": "minecraft:crafting"})));
        assert!(conditions.allows(&json!({"neoforge:conditions": []})));
    }

    #[test]
    fn mod_loaded_follows_the_answer_and_minecraft_always_holds() {
        let conditions = LoadConditions::new(["create"]);
        for (modid, expected) in [("create", true), ("ic2", false), ("minecraft", true)] {
            assert_eq!(
                conditions.allows(&file(
                    json!({"type": "neoforge:mod_loaded", "modid": modid})
                )),
                expected
            );
        }
    }

    #[test]
    fn not_and_or_combine_and_all_must_hold() {
        let conditions = LoadConditions::new(["create"]);
        let absent = json!({"type": "neoforge:mod_loaded", "modid": "ic2"});
        let not = json!({"type": "neoforge:not", "value": absent});
        assert!(conditions.allows(&file(not)));
        assert!(conditions.allows(&json!({"neoforge:conditions": [
            {"type": "neoforge:mod_loaded", "modid": "create"},
            {"type": "neoforge:or", "values": [absent, {"type": "neoforge:always"}]}
        ]})));
        assert!(!conditions.allows(&json!({"neoforge:conditions": [
            {"type": "neoforge:always"}, absent
        ]})));
    }

    #[test]
    fn an_unknown_kind_nested_in_the_tree_leaves_the_file_out() {
        let conditions = LoadConditions::new([]);
        let nested = json!({"type": "neoforge:not", "value":
            {"type": "neoforge:not", "value": {"type": "example:unknown"}}
        });
        let always = json!({"type": "neoforge:always"});
        let never = json!({"type": "neoforge:never"});
        for list in [
            json!([nested]),
            json!([never, nested]),
            json!([{"type":"neoforge:and", "values":[never, nested]}]),
            json!([{"type":"neoforge:or", "values":[always, nested]}]),
        ] {
            let file = json!({"neoforge:conditions": list});
            assert_eq!(
                conditions.allows_file_isolated(&file, "nested.json"),
                Err("unknown load conditions [example:unknown]".into())
            );
        }

        for non_list in [
            Value::Null,
            always.clone(),
            json!("neoforge:always"),
            json!(false),
        ] {
            let file = json!({"neoforge:conditions": non_list});
            assert_eq!(
                conditions.allows_file_isolated(&file, "shape.json"),
                Err("neoforge:conditions: expected a list".into())
            );
        }
        // A malformed node and an unknown ID survive together, even in a dead branch.
        let malformed = json!({"type":"neoforge:or", "values":[always, null, nested]});
        assert!(!conditions.allows_file(&file(malformed), "malformed.json"));
        assert_eq!(
            conditions.check_errors(),
            Err("malformed.json: load condition: missing or invalid type; unknown load conditions [example:unknown]".into())
        );
    }

    fn file(condition: Value) -> Value {
        json!({"neoforge:conditions": [condition]})
    }

    #[test]
    fn always_and_never_evaluate_without_a_registered_handler() {
        let conditions = LoadConditions::new([]);
        assert!(conditions.allows(&file(json!({"type": "neoforge:always"}))));
        assert!(!conditions.allows(&file(json!({"type": "neoforge:never"}))));
        assert!(conditions.allows(&file(
            json!({"type": "neoforge:not", "value": {"type": "neoforge:never"}})
        )));
        assert!(
            conditions.allows(&file(json!({"type": "neoforge:or", "values": [
                {"type": "neoforge:never"}, {"type": "neoforge:always"}
            ]})))
        );
    }

    #[test]
    fn registered_reads_the_loads_registries_and_defaults_to_items() {
        let stone = file(json!({"type": "neoforge:registered", "value": "minecraft:stone"}));
        let missing = file(
            json!({"type": "neoforge:registered", "registry": "minecraft:item", "value": "test:missing"}),
        );
        let biome = file(
            json!({"type": "neoforge:registered", "registry": "worldgen/biome", "value": "plains"}),
        );
        // No registries in this load: unsupported, so the file is left out.
        assert!(!LoadConditions::vanilla().allows(&stone));
        let conditions = LoadConditions::new([]).with_registries([
            ("minecraft:item", vec!["minecraft:stone"]),
            ("minecraft:worldgen/biome", vec!["minecraft:plains"]),
        ]);
        assert!(conditions.allows(&stone));
        assert!(!conditions.allows(&missing));
        assert!(conditions.allows(&biome));
    }

    #[test]
    fn feature_flags_enabled_needs_every_flag() {
        let both = file(json!({"type": "neoforge:feature_flags_enabled",
            "flags": ["minecraft:vanilla", "minecraft:trade_rebalance"]}));
        let vanilla = file(json!({"type": "neoforge:feature_flags_enabled", "flags": ["vanilla"]}));
        assert!(!LoadConditions::vanilla().allows(&vanilla));
        let default = LoadConditions::new([]).with_features([]);
        assert!(default.allows(&vanilla) && !default.allows(&both));
        let experiment = LoadConditions::new([])
            .with_features(["minecraft:vanilla", "minecraft:trade_rebalance"]);
        assert!(experiment.allows(&both));
    }

    #[test]
    fn feature_flag_codec_rejects_unknown_flags_in_unreached_operands() {
        let known_disabled =
            json!({"type":"neoforge:feature_flags_enabled", "flags":["trade_rebalance"]});
        let conditions = LoadConditions::new([]).with_features(["vanilla"]);
        assert_eq!(
            conditions.allows_file_isolated(&file(known_disabled), "known.json"),
            Ok(false)
        );
        let skipped = file(json!({"type":"neoforge:or", "values":[
            {"type":"neoforge:always"},
            {"type":"neoforge:feature_flags_enabled", "flags":["test:typo"]}
        ]}));
        assert!(
            conditions
                .allows_file_isolated(&skipped, "skipped.json")
                .unwrap_err()
                .contains("unknown flag test:typo")
        );
    }

    #[test]
    fn tag_empty_reads_only_the_tags_this_load_bound() {
        let tin = file(json!({"type": "neoforge:tag_empty", "tag": "c:ingots/tin"}));
        let before_tags = LoadConditions::new([]);
        assert!(!before_tags.allows(&tin), "no tag view yet: left out");
        let empty = before_tags.with_tags([("minecraft:item", vec![("minecraft:planks", true)])]);
        assert!(empty.allows(&tin));
        let with_tin = before_tags.with_tags([("minecraft:item", vec![("c:ingots/tin", true)])]);
        assert!(!with_tin.allows(&tin));
        assert_ne!(empty, with_tin, "the tag view is part of cache identity");
    }

    #[test]
    fn an_unsupplied_registry_is_empty_without_a_diagnostic() {
        let conditions = LoadConditions::new([])
            .with_registries([("item", ["stone"])])
            .with_tags([("item", [("planks", true)])]);
        for registry in ["minecraft:worldgen/biome", "test:missing"] {
            let registered = json!({"type": "neoforge:registered",
                "registry": registry, "value": "test:entry"});
            let empty = json!({"type": "neoforge:tag_empty",
                "registry": registry, "tag": "test:tag"});
            assert!(!conditions.allows_file(&file(registered.clone()), "data/test/recipe/a.json"));
            assert!(conditions.allows_file(&file(empty.clone()), "data/test/recipe/b.json"));
            assert!(conditions.allows(&file(json!({"type": "neoforge:not", "value": registered}))));
            assert!(!conditions.allows(&file(json!({"type": "neoforge:not", "value": empty}))));
        }
        assert_eq!(conditions.check_errors(), Ok(()));
    }

    #[test]
    fn a_retained_candidate_does_not_reuse_an_answer_across_tag_views() {
        let tin = file(json!({"type": "neoforge:tag_empty", "tag": "c:ingots/tin"}));
        let candidate = LoadConditions::new([]).with_lookup([("test:flag", |_: &Value| Ok(true))]);
        let empty = candidate.with_tags([("minecraft:item", Vec::<(&str, bool)>::new())]);
        let with_tin = candidate.with_tags([("minecraft:item", vec![("c:ingots/tin", true)])]);
        assert!(empty.allows_file(&tin, "data/a/recipe/b.json"));
        assert!(!with_tin.allows_file(&tin, "data/a/recipe/b.json"));
    }

    #[test]
    fn malformed_builtin_operands_are_errors_before_testing() {
        let conditions = LoadConditions::new([]).with_lookup([("test:unexpected", |_: &Value| {
            panic!("tested before decode")
        })]);
        for malformed in [
            json!({"type": "neoforge:feature_flags_enabled"}),
            json!({"type": "neoforge:feature_flags_enabled", "flags": null}),
            json!({"type": "neoforge:feature_flags_enabled", "flags": "vanilla"}),
            json!({"type": "neoforge:feature_flags_enabled", "flags": [false]}),
            json!({"type": "neoforge:feature_flags_enabled", "flags": ["test:missing", 1]}),
            json!({"type": "neoforge:tag_empty"}),
            json!({"type": "neoforge:tag_empty", "tag": null}),
            json!({"type": "neoforge:tag_empty", "tag": []}),
            json!({"type": "neoforge:tag_empty", "tag": 1}),
            json!({"type": "neoforge:feature_flags_enabled", "flags": []}),
            json!({"type": "neoforge:mod_loaded", "modid": false}),
            json!({"type": "neoforge:registered", "value": true}),
            json!({"type": "neoforge:registered", "registry": null, "value": "stone"}),
            json!({"type": "neoforge:tag_empty", "tag": "bad tag"}),
            json!({"type": "neoforge:not"}),
            json!({"type": "neoforge:and", "values": {}}),
            json!({"type": "neoforge:or", "values": [null]}),
        ] {
            let not = json!({"type": "neoforge:not", "value": malformed.clone()});
            let twice = json!({"type": "neoforge:not", "value": not.clone()});
            for condition in [malformed, not, twice] {
                let file = json!({"neoforge:conditions": [
                    {"type":"test:unexpected"},
                    {"type":"neoforge:or","values":[{"type":"neoforge:always"}, condition]}
                ]});
                assert!(
                    conditions
                        .allows_file_isolated(&file, "invalid.json")
                        .is_err(),
                    "{file}"
                );
            }
        }
    }

    #[test]
    fn decided_branches_skip_handlers_and_unavailable_context() {
        let conditions = LoadConditions::new([])
            .with_lookup([("test:fail", |_: &Value| Err("handler was reached".into()))]);
        let never = json!({"type":"neoforge:never"});
        let always = json!({"type":"neoforge:always"});
        let fail = json!({"type":"test:fail"});
        let missing = json!({"type":"neoforge:not", "value":
            {"type":"neoforge:tag_empty", "tag":"test:x"}
        });
        for tail in [fail.clone(), missing.clone()] {
            for (list, expected) in [
                (json!([never, tail]), false),
                (
                    json!([{"type":"neoforge:and", "values":[never, tail]}]),
                    false,
                ),
                (
                    json!([{"type":"neoforge:or", "values":[always, tail]}]),
                    true,
                ),
            ] {
                let file = json!({"neoforge:conditions": list});
                assert_eq!(
                    conditions.allows_file_isolated(&file, "decided.json"),
                    Ok(expected)
                );
            }
        }
        let undecided = json!({"neoforge:conditions": [always, fail, never]});
        assert_eq!(
            conditions.allows_file_isolated(&undecided, "undecided.json"),
            Err("handler was reached".into())
        );
        assert_eq!(
            conditions.allows_file_isolated(&file(missing), "bootstrap.json"),
            Err("neoforge:tag_empty: tags not loaded yet".into())
        );
        assert_eq!(conditions.check_errors(), Ok(()));
    }

    #[test]
    fn unconditioned_elements_never_build_a_diagnostic_path() {
        let mut entries = vec![json!(1), json!({"name": "test:entry"})];
        let expected = entries.clone();
        LoadConditions::vanilla().filter_list(&mut entries, &|| {
            panic!("an unconditioned element has no diagnostic path")
        });
        assert_eq!(entries, expected);
    }

    #[test]
    fn unconditioned_loot_pools_are_kept_as_written() {
        let mut value = json!({"pools": [{"rolls": 1, "entries": [1, {"name": "test:entry"}]}]});
        let expected = value.clone();
        assert!(LoadConditions::vanilla().admit_loot_table(&mut value, "test:table"));
        assert_eq!(value, expected);
    }

    #[test]
    fn loot_pool_conditions_drop_failing_pools_and_strip_passing_keys() {
        let mut table = json!({"pools": [
            {"rolls": 1},
            {"rolls": 2, "neoforge:conditions": [
                {"type": "neoforge:mod_loaded", "modid": "create"}]},
            {"rolls": 3, "neoforge:conditions": [
                {"type": "neoforge:mod_loaded", "modid": "ic2"}]}
        ]});
        assert!(LoadConditions::new(["create"]).admit_loot_table(&mut table, "a:b"));
        assert_eq!(table["pools"], json!([{"rolls": 1}, {"rolls": 2}]));
    }

    #[test]
    fn an_unknown_condition_does_not_load_the_file() {
        let mut c = serde_json::Map::new();
        c.insert(
            "neoforge:conditions".to_owned(),
            json!([{"type": "neoforge:future_thing"}]),
        );
        assert!(!LoadConditions::vanilla().allows(&Value::Object(c)));
    }
}

#[cfg(test)]
#[path = "condition_registry_tests.rs"]
mod registry_tests;

#[cfg(test)]
#[path = "condition_log_tests.rs"]
mod log_tests;

#[cfg(test)]
#[path = "condition_fabric_tests.rs"]
mod fabric_tests;
