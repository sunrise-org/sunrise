use super::*;

#[derive(Default)]
struct StackProbe {
    stack: Arc<std::sync::OnceLock<std::result::Result<(), &'static str>>>,
}

#[mod_api::sunrise_mod]
impl Hooks for StackProbe {
    fn chunk_data_save(&self, _: &dyn ChunkDiskHost, _: &mut ChunkDataSaveEvent<'_>) {
        self.stack
            .set(sunrise_guest_stack::check_current())
            .unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn asynchronous_chunk_save_hook_runs_with_a_guest_stack() {
    // Subscribed saves consult a process-wide lease. A parallel test must not block this save.
    const CHILD: &str = "SUNRISE_REGION_WRITER_STACK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "save::chunk_disk_tests::writer_stack::asynchronous_chunk_save_hook_runs_with_a_guest_stack",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let probe = StackProbe::default();
    let stack = probe.stack.clone();
    let mods = vec![mod_api::ModEntry::new("stack_probe", Box::new(probe))].into();
    save_chunk(mods);
    assert_eq!(stack.get(), Some(&Ok(())));
}

fn save_chunk(mods: mod_api::Mods) -> i32 {
    let (registry, biomes) = super::super::loot_tests::fixture();
    let registry = Box::leak(Box::new(registry));
    let biomes = Arc::new(biomes);
    let path = tempfile::tempdir().unwrap();
    mod_platform::testing::with_mods(mods, || {
        let save = SaveDir::new(
            path.path().to_path_buf(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        context(&save);
        save.save_later(&[Arc::new(chunk(7))], registry, &biomes)
            .unwrap();
        save.flush().unwrap();
    });
    disk_value(path.path(), registry, &biomes).unwrap()
}
