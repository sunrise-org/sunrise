use super::*;

#[test]
fn interrupted_swap_restores_old_files_and_does_not_consume_the_previous_step() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let interrupted = root.join(JOURNAL);
    fs::create_dir(&interrupted).unwrap();
    fs::create_dir(interrupted.join("old")).unwrap();
    fs::create_dir(interrupted.join("new")).unwrap();
    fs::create_dir(root.join(PREVIOUS_DIR)).unwrap();
    fs::write(
        root.join(PREVIOUS_DIR).join(CONFIG),
        b"before earlier change",
    )
    .unwrap();
    fs::write(interrupted.join("old").join(CONFIG), b"before this change").unwrap();
    fs::write(root.join(CONFIG), b"half swapped").unwrap();
    fs::write(root.join(lock::LOCK), b"never published").unwrap();
    let mut doc = Move {
        old: [false; FILES.len()],
        new: [false; FILES.len()],
        undo: Undo::KeepMods,
    };
    doc.old[0] = true;
    doc.new[0] = true;
    doc.new[1] = true;
    fs::write(
        interrupted.join("move.json"),
        serde_json::to_vec(&doc).unwrap(),
    )
    .unwrap();
    assert!(read_lease(root).is_err());
    let folder = ServerFolder::open(root).unwrap();
    assert!(folder.recover().unwrap());
    assert_eq!(fs::read(root.join(CONFIG)).unwrap(), b"before this change");
    assert!(!root.join(lock::LOCK).exists());
    assert_eq!(
        fs::read(root.join(PREVIOUS_DIR).join(CONFIG)).unwrap(),
        b"before earlier change"
    );
    assert!(!folder.recover().unwrap());
}

#[test]
fn release_changes_keep_separate_undo_and_do_not_revert_runtime_data() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let text = |port| format!("[server]\nport = {port}\n[mods]\nlist = []\n");
    let initial = text(1);
    fs::write(root.join(CONFIG), &initial).unwrap();
    let storage = Path::new(mod_list::DIR).join(sunrise_mod_package::manifest::STORAGE_DIR);
    fs::create_dir_all(root.join(&storage)).unwrap();
    fs::write(root.join(&storage).join("data"), b"first world data").unwrap();
    fs::create_dir(root.join(PREVIOUS_DIR)).unwrap();
    fs::write(root.join(PREVIOUS_DIR).join(lock::LOCK), b"ordinary undo").unwrap();
    fs::write(root.join(lock::LOCK), b"first release mods").unwrap();
    let folder = ServerFolder::open(root).unwrap();
    let stage = folder.stage().unwrap();
    fs::write(stage.path().join(CONFIG), text(1)).unwrap();
    fs::write(stage.path().join(lock::LOCK), b"next release mods").unwrap();
    fs::create_dir_all(stage.path().join(&storage)).unwrap();
    fs::write(stage.path().join(&storage).join("data"), b"new world data").unwrap();
    folder.commit_release(stage, &initial).unwrap();
    assert_eq!(
        fs::read(root.join(lock::LOCK)).unwrap(),
        b"next release mods"
    );
    assert_eq!(
        fs::read(root.join(RELEASE_PENDING).join(lock::LOCK)).unwrap(),
        b"first release mods"
    );
    fs::write(root.join(CONFIG), text(2)).unwrap();
    folder.undo_release(&text(2)).unwrap();
    assert_eq!(
        fs::read(root.join(lock::LOCK)).unwrap(),
        b"first release mods"
    );
    assert_eq!(
        fs::read(root.join(&storage).join("data")).unwrap(),
        b"new world data"
    );
    assert_eq!(fs::read_to_string(root.join(CONFIG)).unwrap(), text(2));
    assert!(!root.join(RELEASE_PENDING).exists());
    assert_eq!(
        fs::read(root.join(PREVIOUS_DIR).join(lock::LOCK)).unwrap(),
        b"ordinary undo"
    );
}
