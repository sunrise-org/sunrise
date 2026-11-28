use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};
use sunrise_client::{
    Result, item_data,
    items::{ItemStack, Items},
};
use sunrise_items::item_nbt;
use sunrise_world::loot::PackOverlay;

fn query(items: &Items, stack: &ItemStack, block: &str) -> String {
    format!(
        "{} {} {} {}",
        items.mining_speed(Some(stack), block).to_bits(),
        items.correct_for_drops(Some(stack), block),
        items
            .mining_damage(stack)
            .map_or_else(|| "none".into(), |n| n.to_string()),
        items.mining_damage_capacity(stack)
    )
}

/// The item table a server builds for its world items with `pack` enabled over the built-in
/// one (`stage_world_tables`): the packs' content files (`DataStore`) over the resource root,
/// through `Items::load_registry`.
fn pack_items(
    root: &Path,
    pack: &Path,
    defaults: &sunrise_world::item_defaults::ItemDefaults,
) -> Result<Items> {
    use sunrise_level::datapack::{DataPack, DataPacks, DataStore};
    let builtin = DataPack::builtin(root);
    let fixture = DataPack::from_source(
        "file/parity".into(),
        sunrise_assets::resource_source::ResourceSource::directory(pack.to_path_buf()),
    );
    let files: Vec<_> = DataStore::load(&DataPacks::new(&[&builtin, &fixture]))
        .files
        .into_iter()
        .collect();
    let (registry, _) = sunrise_world::level::data::load_registry_with_flags(root)?;
    let facts = sunrise_items::blocks::BlockFacts::load(root, &registry)?;
    Items::load_registry(
        root,
        &registry,
        &facts,
        &PackOverlay::new(&files),
        defaults,
        sunrise_items::items::LoadEntityTags::Prepared,
    )
}

pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().collect();
    let root = Path::new(&args[3]);
    let defaults = sunrise_world::item_defaults::read(root)?;
    // The pack rows pass a data pack: the table its world binds.
    let items = match args.get(4) {
        Some(pack) => pack_items(root, Path::new(pack), &defaults)?,
        None => Items::catalog(root, &PackOverlay::NONE, &defaults)?,
    };
    let mut out = BufWriter::new(File::create(&args[2])?);
    for line in BufReader::new(File::open(&args[1])?).lines() {
        let row: serde_json::Value = serde_json::from_str(&line?)?;
        let input = row["item"].as_str().unwrap();
        let (id, components) = input
            .find('[')
            .map_or((input, None), |i| (&input[..i], Some(&input[i..])));
        let mut stack = ItemStack::new(id, 1);
        stack.count = row["count"].as_u64().unwrap_or(1) as u32;
        if let Some(components) = components {
            let tag = item_data::command_components(components)?;
            (stack.damage, stack.extra) = item_data::decode_components(Some(&tag));
        }
        let block = row["block"].as_str().unwrap();
        let restored = if stack.count == 0 || id == "minecraft:air" {
            stack.clone()
        } else {
            item_nbt::item_from_nbt(&item_nbt::item_to_nbt(&stack, &items)).unwrap()
        };
        writeln!(
            out,
            "{} | {}",
            query(&items, &stack, block),
            query(&items, &restored, block)
        )?;
    }
    Ok(())
}
