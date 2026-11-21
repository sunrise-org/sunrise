//! ClientboundUpdateRecipesPacket: what RecipeManager.finalizeRecipeLoading
//! synchronizes, the recipe property sets (the items each furnace, smithing
//! table slot and brewing stand slot accepts) and the stonecutter's recipes,
//! read from the data pack's recipe files.
use super::data::SyncData;
use crate::{crafting::ResolvedRecipe, items::Items};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
    sync::Arc,
};
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::packets::recipes::{HolderSet, SlotDisplay};

/// RecipeManager.RECIPE_PROPERTY_SETS: each set and the ingredients it takes
/// from a recipe of the given type.
pub(crate) const PROPERTY_SETS: &[(&str, &str, &[&str])] = &[
    ("smithing_addition", "smithing", &["addition"]),
    ("smithing_base", "smithing", &["base"]),
    ("smithing_template", "smithing", &["template"]),
    ("furnace_input", "minecraft:smelting", &["ingredient"]),
    ("blast_furnace_input", "minecraft:blasting", &["ingredient"]),
    ("smoker_input", "minecraft:smoking", &["ingredient"]),
    (
        "campfire_input",
        "minecraft:campfire_cooking",
        &["ingredient"],
    ),
    ("brewing_input", "minecraft:brewing", &["input"]),
    ("brewing_reagent", "minecraft:brewing", &["reagent"]),
];

/// An ingredient as its HolderSet: a tag, or the items listed.
enum Ingredient {
    Tag(String),
    Items(Vec<String>),
}

fn ingredient(v: &Value) -> Option<Ingredient> {
    match v {
        Value::String(s) => Some(match s.strip_prefix('#') {
            Some(tag) => Ingredient::Tag(tag.to_owned()),
            None => Ingredient::Items(vec![s.clone()]),
        }),
        Value::Array(a) => Some(Ingredient::Items(
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect(),
        )),
        // A brewing slot: the item predicate's `item` ingredient.
        Value::Object(o) => ingredient(o.get("item")?),
        _ => None,
    }
}

/// The recipes by id in vanilla's registration order (path, then namespace),
/// the order `RecipeManager` numbers groups and lists the stonecutter's
/// recipes in, and so the order the host's own tables keep.
fn registration_order<V>(all: &BTreeMap<String, V>) -> Vec<&V> {
    let mut ordered: Vec<_> = all.iter().collect();
    ordered.sort_by_cached_key(|(id, _)| sunrise_client::crafting::registration_key(id));
    ordered.into_iter().map(|(_, v)| v).collect()
}

/// The network copy of the already-resolved load cohort. Only recipes accepted
/// by the canonical loader participate in display numbering, keeping client
/// display ids aligned with the server's recipe index. Accepted ingredients
/// become flat holder sets, including empty sets, without interpreting custom metadata.
pub(super) fn network_recipes(
    resources: &Path,
    overlay: &sunrise_world::loot::PackOverlay,
    defaults: &sunrise_world::item_defaults::ItemDefaults,
    sources: &sunrise_client::crafting::ResolvedSources,
    damage: sunrise_client_core::registries::RegistryIds,
    packed: Option<&super::data::PackRegistryDefinitions>,
) -> crate::Result<(BTreeMap<String, Box<str>>, HashMap<String, i32>)> {
    let items = sunrise_client::items::Items::catalog_with_damage_registry(
        resources,
        overlay,
        defaults,
        damage,
        |items| match packed {
            Some(packed) => packed.install_items(items, resources, overlay, defaults),
            None => items.load_resource_registries(resources, overlay, defaults),
        },
    )?;
    let accepted =
        sunrise_client::crafting::Recipes::from_resolved(resources, &items, overlay, sources)?;
    let accepted: std::collections::HashSet<&str> = accepted
        .by_type
        .values()
        .flat_map(|recipes| {
            recipes
                .iter()
                .filter(|recipe| recipe.has_placement_candidates())
                .map(|recipe| recipe.id.as_str())
        })
        .collect();
    let accepted_files: BTreeMap<_, _> = sources
        .recipes
        .iter()
        .filter_map(|recipe| {
            let id = sunrise_client::crafting::qualified(&recipe.name);
            accepted.contains(id.as_str()).then_some((id, recipe))
        })
        .collect();
    let all: BTreeMap<_, _> = accepted_files
        .into_iter()
        .filter_map(|(id, recipe)| Some((id, normalize_ingredients(recipe, &items)?)))
        .collect();
    // RecipeManager.unpackRecipeInfo's group numbers, while the files are
    // still parsed.
    let mut groups = HashMap::new();
    for r in registration_order(&all) {
        if let Some(g) = r["group"].as_str().filter(|g| !g.is_empty()) {
            let next = groups.len() as i32;
            groups.entry(g.to_owned()).or_insert(next);
        }
    }
    // Each file as its compact JSON text: a book entry parses the one it
    // needs, and the parsed files would keep about 5 MB alive for the
    // server's whole run.
    let all = all
        .into_iter()
        .map(|(id, r)| (id, r.to_string().into_boxed_str()))
        .collect();
    Ok((all, groups))
}

/// An ingredient object's flat item list: a built-in shape, an occurrence
/// answered by a mod, or a brewing predicate's `item`.
fn flat(recipe: &ResolvedRecipe, ing: &Value, items: &Items) -> Option<Value> {
    if !ing.is_object() {
        return None;
    }
    let value = if ing
        .get("type")
        .or_else(|| ing.get("neoforge:ingredient_type"))
        .is_some()
    {
        ing
    } else {
        // Only an untyped brewing wrapper gives `item` a structural meaning.
        ing.get("item").unwrap_or(ing)
    };
    let parsed = recipe.parse_ingredient(value, items)?;
    Some(Value::Array(
        parsed.order.into_iter().map(Value::String).collect(),
    ))
}

/// Write flat holder sets into a network-only projection. Every read uses
/// the frozen document, where occurrence answers are keyed. `None` excludes
/// a recipe whose ingredients cannot be parsed.
fn normalize_ingredients(recipe: &ResolvedRecipe, items: &Items) -> Option<Value> {
    let file = recipe.file();
    let mut projection = file.clone();
    if !file["type"]
        .as_str()
        .is_some_and(|t| t.starts_with("minecraft:"))
    {
        return Some(projection);
    }
    for key in crate::crafting::INGREDIENT_MAPS {
        let Some(map) = file.get(key).and_then(Value::as_object) else {
            continue;
        };
        for (symbol, ing) in map {
            if let Some(replacement) = flat(recipe, ing, items) {
                projection[key][symbol.as_str()] = replacement;
            } else {
                recipe.parse_ingredient(ing, items)?;
            }
        }
    }
    for key in crate::crafting::INGREDIENT_KEYS {
        let Some(ing) = file.get(key) else { continue };
        if let Some(replacement) = flat(recipe, ing, items) {
            projection[key] = replacement;
        } else if key == "ingredients" {
            for (index, entry) in ing.as_array()?.iter().enumerate() {
                if let Some(replacement) = flat(recipe, entry, items) {
                    projection[key][index] = replacement;
                } else {
                    recipe.parse_ingredient(entry, items)?;
                }
            }
        } else {
            let value = ing.get("item").unwrap_or(ing);
            recipe.parse_ingredient(value, items)?;
        }
    }
    Some(projection)
}

/// For each item, the packet's matching holder-set entries in button order. Values are
/// actual server recipe IDs, not a promise that the item's full stack passes the predicate.
#[derive(Debug, Default, PartialEq)]
pub struct StonecutterButtons(HashMap<Arc<str>, Vec<Arc<str>>>);

impl StonecutterButtons {
    pub fn recipe(&self, input: &str, index: usize) -> Option<&str> {
        self.0.get(input)?.get(index).map(AsRef::as_ref)
    }
}

/// The packet's payload and its stonecutter button map, built from the same projection.
pub fn update_recipes(
    all: &BTreeMap<String, Box<str>>,
    sync: &SyncData,
    sources: &sunrise_client::crafting::ResolvedSources,
) -> (Vec<u8>, StonecutterButtons) {
    // The copy is already filtered and swapped (`network_recipes`); the
    // filters never run twice, so one swap cannot feed a later one again.
    let chosen: Vec<_> = sources
        .recipes
        .iter()
        .filter_map(|recipe| {
            let id = sunrise_client::crafting::qualified(&recipe.name);
            let text = all.get(&id)?;
            Some((id, serde_json::from_str::<Value>(text).ok()?))
        })
        .collect();
    let additions = &sources.property_additions;
    let tags: BTreeMap<&str, &[i32]> = sync
        .tags
        .registries
        .iter()
        .filter(|r| r.registry == "minecraft:item")
        .flat_map(|r| r.tags.iter())
        .map(|t| (t.name.trim_start_matches("minecraft:"), t.ids.as_slice()))
        .collect();
    // An item the registries do not know is left out, not sent as id 0.
    let item = |name: &str| sync.id("minecraft:item", name);
    let items = |i: &Ingredient| -> Vec<i32> {
        match i {
            Ingredient::Tag(t) => tags
                .get(t.trim_start_matches("minecraft:"))
                .map(|ids| ids.to_vec())
                .unwrap_or_default(),
            Ingredient::Items(list) => list.iter().filter_map(|n| item(n)).collect(),
        }
    };
    // A recipe with any required empty ingredient contributes to no property
    // set and never appears in the selectable stonecutter list.
    let placeable = |recipe: &Value| {
        [
            "ingredient",
            "template",
            "base",
            "addition",
            "input",
            "reagent",
        ]
        .into_iter()
        .filter_map(|key| recipe.get(key))
        .all(|value| ingredient(value).is_some_and(|i| !items(&i).is_empty()))
    };
    let mut w = Writer::new();
    let custom: BTreeMap<_, _> = additions.custom().collect();
    w.varint((PROPERTY_SETS.len() + custom.len()) as i32);
    for (key, kind, fields) in PROPERTY_SETS {
        let mut set = BTreeSet::new();
        for (_, recipe) in &chosen {
            let t = recipe["type"].as_str().unwrap_or("");
            let matches = if *kind == "smithing" {
                t.starts_with("minecraft:smithing_")
            } else {
                t == *kind
            };
            if matches && placeable(recipe) {
                for field in *fields {
                    if let Some(i) = ingredient(&recipe[*field]) {
                        set.extend(items(&i));
                    }
                }
            }
        }
        // An item the registries do not know is left out (`item`).
        set.extend(
            additions
                .get(key)
                .into_iter()
                .flatten()
                .filter_map(|n| item(n)),
        );
        w.string(&format!("minecraft:{key}"));
        w.varint(set.len() as i32);
        for id in set {
            w.varint(id);
        }
    }
    for (key, names) in custom {
        let ids: BTreeSet<_> = names.iter().filter_map(|name| item(name)).collect();
        w.string(key);
        w.varint(ids.len() as i32);
        for id in ids {
            w.varint(id);
        }
    }
    // SelectableRecipe.SingleInputSet preserves the resolved source order. The server
    // numbers the same candidate items the packet shows; it still tests the real recipe.
    let mut buttons = StonecutterButtons::default();
    let mut stonecutting = Vec::new();
    let item_names = sync.builtin.get("minecraft:item");
    for (id, recipe) in chosen {
        if recipe["type"] != "minecraft:stonecutting" {
            continue;
        }
        let Some(input) = ingredient(&recipe["ingredient"]) else {
            continue;
        };
        let candidates = items(&input);
        if candidates.is_empty() {
            continue;
        }
        let Some(template) = sunrise_client::crafting::Template::parse(&recipe["result"])
            .filter(|template| template.item.is_some())
        else {
            continue;
        };
        let result = template.create();
        if item(&result.item).is_none() {
            continue;
        }
        let id: Arc<str> = id.into();
        // HolderSets represent membership; an ID only gets this button once.
        for item_id in candidates.iter().copied().collect::<BTreeSet<_>>() {
            if let Some(name) = item_names.and_then(|names| names.name(item_id)) {
                buttons.0.entry(name.into()).or_default().push(id.clone());
            }
        }
        let holders = match input {
            Ingredient::Tag(tag) => HolderSet::Tag(sunrise_client::crafting::qualified(&tag)),
            Ingredient::Items(_) => HolderSet::Items(candidates),
        };
        let stack = super::guest::network_stack(
            &|name| item(name).unwrap_or(0),
            &|registry, name| sync.id(registry, name),
            &result,
        );
        stonecutting.push((holders, SlotDisplay::ItemStack(stack)));
    }
    w.varint(stonecutting.len() as i32);
    for (input, result) in stonecutting {
        input.write(&mut w);
        result.write(&mut w);
    }
    (w.data, buttons)
}

/// The Sunrise channel a server sends its clients the recipes its native mods
/// asked for (`native_datapack_sync`), NeoForge's
/// `OnDatapackSyncEvent.sendRecipes`. Sunrise clients only.
pub const CONTENT_CHANNEL: &str = "sunrise:recipe_content";
/// `ClientboundCustomPayloadPacket`'s cap on a payload, less room for the
/// envelope.
const CONTENT_MAX: usize = 1_048_576 - 1024;

/// One payload of the recipe content: some recipes as `(id, json)`, and on the
/// last one the recipe types they replace on the client (a type the server has
/// no recipe of still empties the client's).
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContentPart {
    pub recipes: Vec<(String, Value)>,
    pub last: Option<Vec<String>>,
    /// Why a recipe of this part has no decodable serializer. Set by
    /// `decode`, never sent: the receiver refuses the whole update when its
    /// last part comes, not only this part.
    #[serde(skip)]
    pub refused: Option<String>,
}
impl ContentPart {
    /// Decode a content part only within the same bound used by the sender.
    /// Each recipe's `type` is decoded here, as on disk, so that the
    /// replacement and the book's numbering read the canonical id even from a
    /// server that sends a bare one. A malformed envelope is an error of this
    /// part; a malformed serializer is `refused`, which keeps `last`.
    pub fn decode(data: &[u8]) -> Result<Self, String> {
        if data.len() > CONTENT_MAX {
            return Err(format!("recipe content exceeds {CONTENT_MAX} bytes"));
        }
        let mut part: Self = serde_json::from_slice(data).map_err(|error| error.to_string())?;
        for (id, recipe) in &mut part.recipes {
            if let Err(error) = sunrise_items::recipe_sources::decode_type(recipe) {
                part.refused = Some(format!("recipe {id}: {error}"));
                break;
            }
        }
        Ok(part)
    }
}

/// The payloads for `types`' recipes among `sources` (`recipe_sources::sources`,
/// vanilla's ids bare), ids made whole (`minecraft:<name>`). With no type
/// asked for: nothing, or with `reset` (this server sent recipes before) one
/// last part with no types, which puts its clients back on their own. A
/// recipe too big for a payload of its own is left out.
pub fn encode_content(types: &[String], sources: &[(String, Value)], reset: bool) -> Vec<Vec<u8>> {
    // The last part carries the types: they count against its cap too.
    let types_len = serde_json::to_string(types).map_or(0, |t| t.len()) + 32;
    if types_len > CONTENT_MAX {
        log::warn!("Sunrise: too many recipe types to send to clients");
        return Vec::new();
    }
    if types.is_empty() && !reset {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut part: Vec<(String, Value)> = Vec::new();
    let mut size = 0;
    for (name, v) in sources {
        if !crate::crafting::mod_kinds::file_has_type(v, types) {
            continue;
        }
        let id = if name.contains(':') {
            name.clone()
        } else {
            format!("minecraft:{name}")
        };
        let len = id.len() + v.to_string().len() + 8;
        if len > CONTENT_MAX {
            log::warn!("Sunrise: recipe {id} is too big to send to clients");
            continue;
        }
        if size + len > CONTENT_MAX {
            let recipes = std::mem::take(&mut part);
            out.push(
                serde_json::to_vec(&ContentPart {
                    recipes,
                    last: None,
                    refused: None,
                })
                .unwrap_or_default(),
            );
            size = 0;
        }
        size += len;
        part.push((id, v.clone()));
    }
    if size + types_len > CONTENT_MAX {
        let recipes = std::mem::take(&mut part);
        out.push(
            serde_json::to_vec(&ContentPart {
                recipes,
                last: None,
                refused: None,
            })
            .unwrap_or_default(),
        );
    }
    let last = ContentPart {
        recipes: part,
        last: Some(types.to_vec()),
        refused: None,
    };
    out.push(serde_json::to_vec(&last).unwrap_or_default());
    out
}

/// One connection's selected recipe types and encoded response. Dropped on disconnect.
#[derive(Default)]
pub struct PlayerRecipeSync {
    pub types: Vec<String>,
    pub payloads: Vec<Vec<u8>>,
    sent: bool,
}

impl PlayerRecipeSync {
    pub fn replace(&mut self, sunrise: bool, mut types: Vec<String>, sources: &[(String, Value)]) {
        types.sort();
        types.dedup();
        self.payloads = if sunrise {
            encode_content(&types, sources, self.sent)
        } else {
            Vec::new()
        };
        self.sent |= !self.payloads.is_empty();
        self.types = types;
    }
}

/// Sunrise recipe viewer transfer channel.
pub const TRANSFER_CHANNEL: &str = "sunrise:recipe_transfer";

/// Decode a transfer request: container, bounded recipe id, optional all flag.
pub fn decode_transfer_request(data: &[u8]) -> sunrise_protocol::Result<(i32, String, bool)> {
    let mut reader = sunrise_protocol::buf::Reader::new(data);
    let container = reader.varint()?;
    let id = reader.string(512)?;
    let all = match reader.remaining() {
        0 => false,
        1 => reader.u8()? == 1,
        _ => {
            return Err(sunrise_protocol::Error::new(
                "recipe transfer request: trailing bytes",
            ));
        }
    };
    Ok((container, id, all))
}

/// Decode a fixed-shape transfer result, or an empty feature advertisement.
pub fn decode_transfer_reply(data: &[u8]) -> sunrise_protocol::Result<Option<(i32, u8)>> {
    if data.is_empty() {
        return Ok(None);
    }
    let mut reader = sunrise_protocol::buf::Reader::new(data);
    let reply = (reader.varint()?, reader.u8()?);
    if reader.remaining() != 0 {
        return Err(sunrise_protocol::Error::new(
            "recipe transfer reply: trailing bytes",
        ));
    }
    Ok(Some(reply))
}

/// Sunrise's check of a viewer transfer, worked out without moving anything
/// (`Menu::check_transfer`); like `TRANSFER_CHANNEL`, an empty clientbound
/// payload advertises it. A request is the menu id, a tag the client picks and
/// the recipe's id; the reply is the menu id, the tag, the result as
/// `TRANSFER_CHANNEL` sends it (`result_byte`) and, for `Missing`, the count
/// of the recipe's cells nothing fills then each cell as a byte.
pub const CHECK_CHANNEL: &str = "sunrise:recipe_check";

/// Whether `channel` is one of the recipe viewer's (`TRANSFER_CHANNEL`,
/// `CHECK_CHANNEL`).
pub fn viewer_channel(channel: &str) -> bool {
    channel == TRANSFER_CHANNEL || channel == CHECK_CHANNEL
}

/// Tells a Sunrise client the server answers both viewer channels: an empty
/// payload on each.
pub fn advertise_viewer_channels(g: &super::guest::Guest) {
    for channel in [TRANSFER_CHANNEL, CHECK_CHANNEL] {
        g.send(
            &sunrise_protocol::packets::play::Clientbound::CustomPayload(
                sunrise_protocol::packets::configuration::CustomPayload {
                    channel: channel.into(),
                    data: Vec::new(),
                },
            ),
        );
    }
}

/// A viewer transfer's result as the engine's menu gives it.
pub fn transfer_result(
    result: Result<(), crate::menu::place::TransferError>,
) -> mod_api::RecipeTransferResult {
    use crate::menu::place::TransferError;
    use mod_api::RecipeTransferResult as R;
    match result {
        Ok(()) => R::Moves,
        Err(TransferError::Missing) => R::Missing,
        Err(TransferError::NoRoom) => R::NoRoom,
        Err(TransferError::Unavailable) => R::NotIntoMenu,
    }
}

/// A `menu_transfer` part's answer as the engine's transfer result (`transfer_result` back).
pub fn transfer_outcome(
    result: mod_api::RecipeTransferResult,
) -> Result<(), crate::menu::place::TransferError> {
    use crate::menu::place::TransferError;
    use mod_api::RecipeTransferResult as R;
    match result {
        R::Moves => Ok(()),
        R::Missing => Err(TransferError::Missing),
        R::NoRoom => Err(TransferError::NoRoom),
        R::NotIntoMenu => Err(TransferError::Unavailable),
    }
}

/// The byte both transfer channels carry a result as.
pub fn result_byte(result: mod_api::RecipeTransferResult) -> u8 {
    use mod_api::RecipeTransferResult as R;
    match result {
        R::Moves => 0,
        R::Missing => 1,
        R::NoRoom => 2,
        R::NotIntoMenu => 3,
    }
}

/// The result a transfer channel's byte names; None for any other byte.
pub fn byte_result(byte: u8) -> Option<mod_api::RecipeTransferResult> {
    use mod_api::RecipeTransferResult as R;
    Some(match byte {
        0 => R::Moves,
        1 => R::Missing,
        2 => R::NoRoom,
        3 => R::NotIntoMenu,
        _ => return None,
    })
}

/// The recipe content the native mods asked for (stored on this guest),
/// to one Sunrise client: none when no mod asked.
pub fn send_content(g: &super::guest::Guest) {
    if g.brand != "sunrise" {
        return;
    }
    for data in &g.recipe_sync.payloads {
        g.send(
            &sunrise_protocol::packets::play::Clientbound::CustomPayload(
                sunrise_protocol::packets::configuration::CustomPayload {
                    channel: CONTENT_CHANNEL.into(),
                    data: data.clone(),
                },
            ),
        );
    }
}

#[cfg(test)]
mod content_tests {
    use super::*;
    use serde_json::json;
    use sunrise_client::crafting::ResolvedSources;

    use mod_api::DataLoadOrigin;
    use sunrise_world::native_load_context::{
        DataLoadOrigin as EngineDataLoadOrigin, LoadContext, LoadInputs,
    };

    #[test]
    fn built_in_ingredient_network_lists_use_membership_not_display() {
        let recipe = ResolvedRecipe::fixture(
            "fixture:display".into(),
            Arc::new(json!({
                "type": "minecraft:crafting_shapeless",
                "ingredients": [
                    {"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5}},
                    {"type":"neoforge:custom_display","base":"minecraft:apple","display":{"type":"minecraft:item","item":"minecraft:diamond"}}
                ],
                "result": {"id":"minecraft:emerald"}
            })),
        );
        let recipe = normalize_ingredients(&recipe, &Items::empty()).unwrap();
        assert_eq!(
            recipe["ingredients"],
            json!([["minecraft:diamond_sword"], ["minecraft:apple"]])
        );
    }

    #[derive(Default)]
    struct AspectCalls {
        load: std::sync::Mutex<Vec<(DataLoadOrigin, String)>>,
        matched: std::sync::Mutex<Vec<(String, &'static str, &'static str)>>,
    }
    struct AspectListener(std::sync::Arc<AspectCalls>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for AspectListener {
        fn ingredient(
            &self,
            context: &dyn mod_api::DataLoadContext,
            event: &mut mod_api::IngredientEvent<'_>,
        ) -> Option<mod_api::IngredientDefinition> {
            let kind = event.kind;
            let data = event.data;
            if kind != "mymod:aspect" {
                return None;
            }
            let sunrise_core::nbt::Tag::String(aspect) = data.get(&"aspect".into()).unwrap() else {
                panic!("expected aspect string");
            };
            let aspect = aspect.to_string().unwrap();
            let count = match aspect.as_str() {
                "nested" => 2,
                "plain" => 3,
                "banner" => 4,
                _ => 1,
            };
            self.0
                .load
                .lock()
                .unwrap()
                .push((context.origin(), aspect.clone()));
            let calls = self.0.clone();
            Some(mod_api::IngredientDefinition::new(
                vec![
                    "minecraft:emerald".into(),
                    "minecraft:lapis_lazuli".into(),
                    "minecraft:emerald".into(),
                ],
                move |_, stack| {
                    let call = mod_api::content::crash::current_native_call().unwrap();
                    calls
                        .matched
                        .lock()
                        .unwrap()
                        .push((aspect.clone(), call.id, call.hook));
                    stack.count() == count
                },
            ))
        }
    }

    struct ShadowedAspect;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ShadowedAspect {
        fn ingredient(
            &self,
            _: &dyn mod_api::DataLoadContext,
            _: &mut mod_api::IngredientEvent<'_>,
        ) -> Option<mod_api::IngredientDefinition> {
            panic!("later mod was asked after the first ingredient answer");
        }
    }

    fn load(origin: EngineDataLoadOrigin) -> LoadContext {
        LoadContext::new(
            origin,
            std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
            [],
        )
    }

    /// The `ingredient` hook hears each custom-ingredient occurrence once per
    /// load, in the load's own pass (`ResolvedSources::resolve`), whatever
    /// reads the recipes afterwards: every parse and the guests' copy read the
    /// answer the pass left. The occurrence under a slot other than
    /// `ingredients` (a banner recipe's `banner`) and the nested one are
    /// answered too. Source JSON remains intact but cannot supply an answer.
    #[test]
    fn ingredient_hook_hears_each_occurrence_once_per_load_with_its_origin() {
        let calls = std::sync::Arc::new(AspectCalls::default());
        let mods = vec![
            mod_api::ModEntry::new("aspects", Box::new(AspectListener(calls.clone()))),
            mod_api::ModEntry::new("shadow", Box::new(ShadowedAspect)),
        ];
        let items = sunrise_client::items::Items::for_tests(Default::default());
        let recipe = json!({
            "type": "minecraft:crafting_shapeless",
            "ingredients": [
                {"type": "neoforge:compound", "children": [
                    {"type": "mymod:aspect", "aspect": "nested"},
                    {"type": "mymod:aspect", "aspect": "plain"}, "minecraft:stone"]}
            ],
            "banner": {"type": "mymod:aspect", "aspect": "banner",
                "sunrise:resolved_items": ["minecraft:diamond"]},
            "result": {"id": "minecraft:stick", "count": 1}
        });
        let sources = || vec![("aspects_test".to_owned(), recipe.clone())];
        mod_platform::testing::with_mods(mods.into(), || {
            let server_origin = DataLoadOrigin::ServerDataLoad;
            let resolved =
                ResolvedSources::resolve(sources(), &load(EngineDataLoadOrigin::ServerDataLoad))
                    .unwrap();
            let once = calls.load.lock().unwrap().clone();
            assert_eq!(
                once,
                [
                    (server_origin, "nested".to_owned()),
                    (server_origin, "plain".to_owned()),
                    (server_origin, "banner".to_owned()),
                ]
            );

            let loaded = &resolved.recipes[0];
            let file = loaded.file();
            assert_eq!(file, &recipe);
            let banner = loaded.parse_ingredient(&file["banner"], &items).unwrap();
            assert_eq!(
                banner.order,
                ["minecraft:emerald", "minecraft:lapis_lazuli"]
            );
            let context = load(EngineDataLoadOrigin::ServerDataLoad)
                .loop_context(sunrise_world::native_load_context::LoopSide::Server, []);
            let nested = loaded
                .parse_ingredient(&file["ingredients"][0], &items)
                .unwrap();
            use sunrise_items::items::ItemStack;
            assert!(nested.test_in(&context, &ItemStack::new("minecraft:emerald", 2)));
            assert!(nested.test_in(&context, &ItemStack::new("minecraft:emerald", 3)));
            assert!(!nested.test_in(&context, &ItemStack::new("minecraft:diamond", 3)));
            {
                let mut matched = calls.matched.lock().unwrap();
                assert_eq!(
                    *matched,
                    [
                        ("nested".to_owned(), "aspects", "ingredient_predicate"),
                        ("nested".to_owned(), "aspects", "ingredient_predicate"),
                        ("plain".to_owned(), "aspects", "ingredient_predicate"),
                    ]
                );
                matched.clear();
            }
            assert_eq!(
                nested.test_in_guarded(&context, &ItemStack::new("minecraft:emerald", 3), &|| {
                    calls.matched.lock().unwrap().is_empty()
                },),
                None
            );
            assert_eq!(
                *calls.matched.lock().unwrap(),
                [("nested".to_owned(), "aspects", "ingredient_predicate"),]
            );
            // Neither standalone parsing nor an identical JSON value outside
            // this document can supply the forged or the real answer.
            assert!(crate::crafting::Ingredient::parse(&file["banner"], &items).is_none());
            assert!(loaded.parse_ingredient(&recipe["banner"], &items).is_none());

            let root = tempfile::tempdir().unwrap();
            let tables = crate::crafting::Recipes::from_resolved(
                root.path(),
                &items,
                &sunrise_world::loot::PackOverlay::NONE,
                &resolved,
            )
            .unwrap();
            assert_eq!(&*tables.by_type["minecraft:crafting"][0].json, &recipe);

            let copy = normalize_ingredients(loaded, &items).unwrap();
            assert_eq!(
                copy,
                json!({
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [["minecraft:emerald", "minecraft:lapis_lazuli", "minecraft:stone"]],
                    "banner": ["minecraft:emerald", "minecraft:lapis_lazuli"],
                    "result": {"id": "minecraft:stick", "count": 1}
                })
            );
            assert_eq!(loaded.file(), &recipe);
            assert_eq!(*calls.load.lock().unwrap(), once);

            calls.load.lock().unwrap().clear();
            let client =
                ResolvedSources::resolve(sources(), &load(EngineDataLoadOrigin::ClientPreload))
                    .unwrap();
            assert!(
                calls
                    .load
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|(origin, _)| *origin == DataLoadOrigin::ClientPreload)
            );
            assert_eq!(calls.load.lock().unwrap().len(), 3);
            drop(client);
        });
    }

    struct PropertyListener(std::sync::Arc<std::sync::Mutex<Vec<DataLoadOrigin>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for PropertyListener {
        fn recipe_property_sets(
            &self,
            context: &dyn mod_api::DataLoadContext,
            event: &mut mod_api::RecipePropertySetsEvent<'_>,
        ) {
            self.0.lock().unwrap().push(context.origin());
            let add = |set: &str, items: &[&str]| {
                mod_api::RecipePropertyAddition::new(
                    set,
                    items.iter().map(|i| (*i).to_owned()).collect(),
                )
            };
            event.additions.push(add(
                "minecraft:furnace_input",
                &["minecraft:emerald", "minecraft:air"],
            ));
            event
                .additions
                .push(add("minecraft:no_such_set", &["minecraft:dirt"]));
            event
                .additions
                .push(add("brewing_reagent", &["minecraft:diamond"]));
        }
    }

    /// The `recipe_property_sets` hook is asked once per load with the load's
    /// origin; the resolved set keeps the known sets' items (a set key with or
    /// without `minecraft:`), drops air and unknown sets, and the furnace
    /// takes what a mod added though no recipe names it.
    #[test]
    fn property_sets_are_asked_once_per_load_and_validated() {
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "props",
            Box::new(PropertyListener(heard.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            let resolved =
                ResolvedSources::resolve(Vec::new(), &load(EngineDataLoadOrigin::ServerDataLoad))
                    .unwrap();
            assert_eq!(*heard.lock().unwrap(), [DataLoadOrigin::ServerDataLoad]);
            let added = &resolved.property_additions;
            assert!(added.adds("furnace_input", "minecraft:emerald"));
            assert!(!added.adds("furnace_input", "minecraft:air"));
            assert!(added.adds("brewing_reagent", "minecraft:diamond"));
            assert!(added.get("no_such_set").is_none());
            assert!(!added.adds("smoker_input", "minecraft:emerald"));
        });
    }

    /// Every slot `Recipes`, `StationRecipes` and the guests' copy read an
    /// ingredient from (the keys their readers name, listed here from them),
    /// the shaped `key` and the firework star's `shapes` maps among them,
    /// carries a mod ingredient that parses after the pass: none fails where
    /// it worked before the pass.
    #[test]
    fn every_slot_the_readers_use_is_answered_by_the_pass() {
        let calls = std::sync::Arc::new(AspectCalls::default());
        let mods = vec![mod_api::ModEntry::new(
            "aspects",
            Box::new(AspectListener(calls.clone())),
        )];
        let items = sunrise_client::items::Items::for_tests(Default::default());
        let slots = [
            "ingredient",
            "ingredients",
            "base",
            "addition",
            "template",
            "input",
            "reagent",
            "banner",
            "target",
            "material",
            "dye",
            "source",
            "shell",
            "fuel",
            "star",
            "trail",
            "twinkle",
            "map",
            "back",
            "left",
            "right",
            "front",
        ];
        let mut recipe = json!({"type": "minecraft:crafting_special_firework_star"});
        for slot in slots {
            recipe[slot] = json!({"type": "mymod:aspect", "aspect": slot});
        }
        recipe["reagent"] = json!({"item": recipe["reagent"]});
        recipe["key"] = json!({"a": {"type": "mymod:aspect", "aspect": "key"}});
        recipe["shapes"] = json!({"star": {"type": "mymod:aspect", "aspect": "shapes"}});
        mod_platform::testing::with_mods(mods.into(), || {
            let resolved = ResolvedSources::resolve(
                vec![("slots".to_owned(), recipe.clone())],
                &load(EngineDataLoadOrigin::ServerDataLoad),
            )
            .unwrap();
            assert_eq!(calls.load.lock().unwrap().len(), slots.len() + 2);
            let loaded = &resolved.recipes[0];
            let file = loaded.file();
            let parse = |ingredient: &Value| {
                loaded
                    .parse_ingredient(ingredient, &items)
                    .unwrap_or_else(|| panic!("{ingredient} fails to parse"))
            };
            for slot in slots {
                let value = &file[slot];
                parse(value.get("item").unwrap_or(value));
            }
            parse(&file["key"]["a"]);
            parse(&file["shapes"]["star"]);
            let projected = normalize_ingredients(loaded, &items).unwrap();
            assert_eq!(
                projected["reagent"],
                json!(["minecraft:emerald", "minecraft:lapis_lazuli"])
            );
            assert_eq!(
                projected["key"]["a"],
                json!(["minecraft:emerald", "minecraft:lapis_lazuli"])
            );
            assert_eq!(
                projected["shapes"]["star"],
                json!(["minecraft:emerald", "minecraft:lapis_lazuli"])
            );
            assert_eq!(calls.load.lock().unwrap().len(), slots.len() + 2);
        });
    }

    #[test]
    fn content_decode_bounds_before_deserializing() {
        let mut body = br#"{"recipes":[],"last":null}"#.to_vec();
        body.resize(1_047_552, b' ');
        assert_eq!(ContentPart::decode(&body).unwrap().recipes, vec![]);
        body.push(b' ');
        assert!(ContentPart::decode(&body).unwrap_err().contains("exceeds"));
    }

    #[test]
    fn content_keeps_the_asked_types_and_ends_with_them() {
        mod_platform::testing::with_recipe_types(&["mymod:crusher"], || {
            let sources = vec![
                ("furnace".to_owned(), json!({"type": "minecraft:smelting"})),
                (
                    "stick".to_owned(),
                    json!({"type": "minecraft:crafting_shaped"}),
                ),
                ("mymod:crush".to_owned(), json!({"type": "mymod:crusher"})),
            ];
            assert!(encode_content(&[], &sources, false).is_empty());
            // After a sync that sent recipes, asking for none sends an empty last part.
            let reset = encode_content(&[], &sources, true);
            assert_eq!(reset.len(), 1);
            let part: ContentPart = serde_json::from_slice(&reset[0]).unwrap();
            assert!(part.recipes.is_empty());
            assert_eq!(part.last.as_deref(), Some(&[][..]));
            let types = vec![
                "minecraft:smelting".to_owned(),
                "mymod:crusher".to_owned(),
                "x:none".to_owned(),
            ];
            let parts = encode_content(&types, &sources, false);
            assert_eq!(parts.len(), 1);
            let part: ContentPart = serde_json::from_slice(&parts[0]).unwrap();
            let ids: Vec<&str> = part.recipes.iter().map(|(id, _)| id.as_str()).collect();
            assert_eq!(ids, ["minecraft:furnace", "mymod:crush"]);
            assert_eq!(part.last.as_deref(), Some(types.as_slice()));
        });
    }

    #[test]
    fn content_splits_under_the_payload_cap() {
        mod_platform::testing::with_recipe_types(&["m:t"], || {
            let big = "x".repeat(300_000);
            let sources: Vec<_> = (0..10)
                .map(|i| (format!("m:r{i}"), json!({"type": "m:t", "pad": big})))
                .collect();
            let parts = encode_content(&["m:t".to_owned()], &sources, false);
            assert!(parts.len() >= 3);
            let mut n = 0;
            for (i, p) in parts.iter().enumerate() {
                assert!(p.len() <= 1_048_576);
                let part: ContentPart = serde_json::from_slice(p).unwrap();
                assert_eq!(part.last.is_some(), i + 1 == parts.len());
                n += part.recipes.len();
            }
            assert_eq!(n, 10);
        });
    }

    #[test]
    fn a_long_type_list_keeps_the_last_part_under_the_cap() {
        mod_platform::testing::with_recipe_types(&["mod0:type_with_a_long_name"], || {
            let types: Vec<String> = (0..5000)
                .map(|i| format!("mod{i}:type_with_a_long_name"))
                .collect();
            let big = "x".repeat(1_000_000);
            let sources = vec![(
                "m:r".to_owned(),
                json!({"type": "mod0:type_with_a_long_name", "pad": big}),
            )];
            let parts = encode_content(&types, &sources, false);
            assert_eq!(parts.len(), 2);
            assert!(parts.iter().all(|p| p.len() <= 1_048_576));
            let last: ContentPart = serde_json::from_slice(&parts[1]).unwrap();
            assert!(last.recipes.is_empty());
            assert_eq!(last.last.map(|t| t.len()), Some(5000));
        });
    }
}

#[cfg(test)]
mod per_player_tests {
    use super::*;

    #[test]
    fn per_player_content_keeps_grinding_private_and_resets_only_its_owner() {
        mod_platform::testing::with_recipe_types(&["testmod:grinding"], || {
            let sources = vec![(
                "testmod:ore".into(),
                serde_json::json!({"type":"testmod:grinding", "result":"minecraft:iron_ingot"}),
            )];
            let mut a = PlayerRecipeSync::default();
            let mut b = PlayerRecipeSync::default();
            let mut vanilla = PlayerRecipeSync::default();
            a.replace(true, vec!["testmod:grinding".into()], &sources);
            b.replace(true, vec![], &sources);
            vanilla.replace(false, vec!["testmod:grinding".into()], &sources);
            let part = ContentPart::decode(&a.payloads[0]).unwrap();
            assert_eq!(part.recipes, sources);
            assert_eq!(part.last, Some(vec!["testmod:grinding".into()]));
            assert!(b.payloads.is_empty());
            assert!(vanilla.payloads.is_empty());
            a.replace(true, vec![], &sources);
            b.replace(true, vec!["testmod:grinding".into()], &sources);
            vanilla.replace(false, vec!["testmod:grinding".into()], &sources);
            assert_eq!(
                ContentPart::decode(&a.payloads[0]).unwrap().last,
                Some(vec![])
            );
            assert_eq!(
                ContentPart::decode(&b.payloads[0]).unwrap().recipes,
                sources
            );
            assert!(vanilla.payloads.is_empty());
        });
    }
}

#[cfg(test)]
#[test]
fn empty_ingredient_network_consumers_exclude_whole_unplaceable_recipes() {
    use serde_json::json;
    use sunrise_protocol::{
        buf::{Codec, Reader},
        packets::recipes::UpdateRecipes,
        registry::Ids,
    };
    struct EmptyPlacement;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for EmptyPlacement {
        fn ingredient(
            &self,
            _: &dyn mod_api::DataLoadContext,
            event: &mut mod_api::IngredientEvent<'_>,
        ) -> Option<mod_api::IngredientDefinition> {
            let kind = event.kind;
            (kind == "fixture:predicate")
                .then(|| mod_api::IngredientDefinition::new(vec![], |_, _| true))
        }
    }

    let mut catalog = Items::for_tests(Default::default());
    catalog
        .item_tag_order
        .insert("fixture:empty".into(), vec![]);
    let mut sync = SyncData::empty();
    sync.builtin.registries.insert(
        "minecraft:item".into(),
        Ids::from_names(vec![
            "minecraft:air".into(),
            "minecraft:stone".into(),
            "minecraft:stick".into(),
        ]),
    );
    for (source, retained) in [
        (json!([]), false),
        (json!("#fixture:missing"), false),
        (json!("#fixture:empty"), true),
        (
            json!({"type":"fixture:predicate", "item":"minecraft:stone"}),
            true,
        ),
    ] {
        let mut files: Vec<_> = [
            "minecraft:stonecutting",
            "minecraft:smelting",
            "minecraft:smithing_transform",
        ]
        .into_iter()
        .map(|kind| {
            (
                kind.to_owned(),
                json!({"type":kind, "ingredient":source,
                "base":"minecraft:stone", "addition":source, "result":{"id":"minecraft:stick"}}),
            )
        })
        .collect();
        files.push((
            "fixture:placeable".into(),
            json!({
                "type":"minecraft:stonecutting", "ingredient":"minecraft:stone",
                "result":{"id":"minecraft:stick"}
            }),
        ));
        let sources = mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("fixture", Box::new(EmptyPlacement))].into(),
            || {
                crate::crafting::ResolvedSources::resolve(
                    files,
                    &crate::test_resources::server_load(),
                )
                .unwrap()
            },
        );
        let all = sources
            .recipes
            .iter()
            .filter_map(|recipe| {
                let projected = normalize_ingredients(recipe, &catalog);
                if recipe.name != "fixture:placeable" {
                    assert_eq!(projected.is_some(), retained);
                }
                let projected = projected?;
                // `item` on a typed predicate is metadata, not a candidate.
                if source.is_object() && recipe.name != "fixture:placeable" {
                    assert_eq!(projected["ingredient"], json!([]));
                }
                Some((recipe.name.clone(), projected.to_string().into_boxed_str()))
            })
            .collect();
        let (bytes, buttons) = update_recipes(&all, &sync, &sources);
        let packet = UpdateRecipes::read(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(
            packet.stonecutter,
            [(
                HolderSet::Items(vec![1]),
                SlotDisplay::ItemStack(sunrise_protocol::item::Stack::new(2, 1)),
            )]
        );
        assert_eq!(
            buttons.recipe("minecraft:stone", 0),
            Some("fixture:placeable")
        );
        assert_eq!(buttons.recipe("minecraft:stone", 1), None);
        assert!(packet.properties.iter().all(|(_, ids)| ids.is_empty()));
    }
}
