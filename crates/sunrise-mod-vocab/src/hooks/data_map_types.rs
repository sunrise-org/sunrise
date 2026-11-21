//! The `data_map_types` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0150

pub trait Hooks {
    /// The server is assembling its data tables, as for
    /// `data_pack_files` (a world's first open and every `/reload`):
    /// push the data map types your mod registers (NeoForge's
    /// `RegisterDataMapTypesEvent`). A type is an `id` (`mod_id:name`, not in
    /// `minecraft`) for a `registry`: any the game names. Its values come
    /// from the world's data packs'
    /// `data/<mod_id>/data_maps/<registry folder>/<name>.json` (the registry's
    /// path for a `minecraft` registry, `<namespace>/<path>` otherwise), and
    /// from files you push through `native_data_pack_files` under the key
    /// `data_maps/<mod_id>:<registry folder>/<name>`: `replace`, `values`
    /// (entry ids and `#tags` to JSON values, each optionally guarded by its
    /// own `neoforge:conditions`) and `remove`, a newer value overriding an
    /// older. Push a typed map's `DataMap::ty` and use the key's `data`
    /// (`ItemKey::data`, `EntityTypeKey::data`...) for an owned decoded value.
    /// By name, `data_map_value` borrows a raw tag and `data_map_entries`
    /// borrows the whole slice in entry ID order. Both borrow from the host;
    /// copy values only if you need them after the borrow. Server-side, not
    /// synced to clients.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_data_map_types",
        dispatch = "flat",
        side = "server",
        contract = "collect",
        out = "types",
        neoforge = "RegisterDataMapTypesEvent"
    )]
    fn data_map_types(types: &mut Vec<DataMapType>);
}
