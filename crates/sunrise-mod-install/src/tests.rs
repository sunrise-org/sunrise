use crate::{
    Side,
    index::{self, Index, Kind, Project},
    input::{ModId, ModRequest, Mods},
    lock::{ModLock, Reason},
    registry::Fetch,
    selection::{self, Refresh, Target},
    transaction::{self, ServerFolder},
};
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read},
};
use sunrise_mod_package::{Package, mod_list};

#[derive(Default)]
struct Files(BTreeMap<String, Vec<u8>>);
impl Fetch for Files {
    fn open(&self, url: &str) -> Result<Box<dyn Read>, String> {
        self.0
            .get(url)
            .cloned()
            .map(|bytes| Box::new(Cursor::new(bytes)) as Box<dyn Read>)
            .ok_or_else(|| format!("unexpected request: {url}"))
    }
}

fn registry(rows: &[(&str, &str, &str, &str)]) -> (Index, Files) {
    let dir = tempfile::tempdir().unwrap();
    let mut projects = BTreeMap::<_, Project>::new();
    let mut fetch = Files::default();
    for &(id, version, side, deps) in rows {
        let text = format!(
            "id='{id}'\nname='{id}'\nversion='{version}'\napi='0.2'\nside='{side}'\n{deps}\n"
        );
        let bytes = Package::write_files(&text, &[]).unwrap();
        let path = dir.path().join("test.smod");
        fs::write(&path, &bytes).unwrap();
        let facts = index::inspect(&Kind::Mod, &path).unwrap();
        let url = format!("https://example.org/{id}/{version}.smod");
        let sha256 = index::sha256(&bytes);
        let size = bytes.len();
        fetch.0.insert(url.clone(), bytes);
        let project = projects.entry(id).or_insert_with(|| {
            serde_json::from_value(serde_json::json!({
                "id": id, "name": id, "summary": id, "kind": "mod",
            }))
            .unwrap()
        });
        project.versions.push(serde_json::from_value(serde_json::json!({
            "version": version, "api": facts.api, "side": facts.side, "game": "26.3", "dependencies": facts.dependencies,
            "capabilities": facts.capabilities,
            "file": { "name": format!("{id}.smod"), "url": url, "sha256": sha256, "size": size },
        })).unwrap());
    }
    (
        Index {
            schema: 1,
            projects: projects.into_values().collect(),
            ..Index::default()
        },
        fetch,
    )
}

fn target() -> Target<'static> {
    Target {
        game: "26.3",
        build: "a".repeat(40).try_into().unwrap(),
        release: None,
        side: Side::Server,
    }
}

fn input(rows: &[&str]) -> Mods {
    Mods {
        list: rows
            .iter()
            .map(|id| ModRequest::try_from((*id).to_owned()).unwrap())
            .collect(),
        ..Mods::default()
    }
}

#[test]
fn server_resolves_ranges_and_skips_client_pack_members() {
    let (registry, _) = registry(&[
        ("rails", "2.0.0", "both", ""),
        ("rails", "1.2.0", "both", "[dependencies]\nbeams='^2'"),
        ("beams", "2.0.0", "both", ""),
        ("overlay", "1.0.0", "client", ""),
    ]);
    let pack = crate::modpack::Manifest {
        format: 1,
        id: "cozy".into(),
        name: "Cozy".into(),
        version: "1.0.0".into(),
        game: "26.3".into(),
        authors: vec![],
        description: String::new(),
        mods: vec![crate::modpack::PackMod {
            id: "overlay".into(),
            version: "1.0.0".into(),
            sha256: registry.project("overlay").unwrap().versions[0]
                .file
                .sha256
                .clone(),
            on: true,
        }],
    };
    let goal = target();
    let plan = selection::plan(
        &registry,
        &goal,
        input(&["rails@^1"]),
        Some(&pack),
        None,
        Refresh::None,
        |_, _| None,
    )
    .unwrap();
    assert_eq!(plan.skipped_pack_members, ["overlay"]);
    assert_eq!(plan.lock.mods[1].version.to_string(), "1.2.0");
    assert_eq!(plan.lock.mods[0].reason, Reason::Required);
    assert_eq!(plan.lock.mods[0].required_by[0].as_str(), "rails");
    let mut disable = plan.lock.input.clone();
    disable
        .off
        .push(ModId::try_from("beams".to_owned()).unwrap());
    let error = selection::plan(
        &registry,
        &goal,
        disable,
        Some(&pack),
        Some(&plan.lock),
        Refresh::None,
        |_, _| None,
    )
    .err()
    .unwrap();
    assert!(error.contains("rails needs beams"), "{error}");
    let error = selection::plan(
        &registry,
        &goal,
        input(&["overlay"]),
        None,
        None,
        Refresh::None,
        |_, _| None,
    )
    .err()
    .unwrap();
    assert!(error.contains("client-only"), "{error}");
}

#[test]
fn canonical_pack_pins_select_the_same_package_as_an_abbreviated_registry_label() {
    let (index, _) = registry(&[("rails", "1.2", "both", "")]);
    let hash = &index.projects[0].versions[0].file.sha256;
    let mut pack = crate::modpack::Manifest {
        format: 1,
        id: "server_mods".into(),
        name: "Server mods".into(),
        version: "1.0.0".into(),
        game: "26.3".into(),
        authors: vec![],
        description: String::new(),
        mods: vec![crate::modpack::PackMod {
            id: "rails".into(),
            version: "1.2.0".into(),
            sha256: hash.clone(),
            on: true,
        }],
    };
    let plan = |pack: &crate::modpack::Manifest| {
        selection::plan(
            &index,
            &target(),
            Mods::default(),
            Some(pack),
            None,
            Refresh::None,
            |_, _| None,
        )
    };
    let lock = plan(&pack).unwrap().lock;
    assert_eq!(lock.mods[0].version.to_string(), "1.2.0");
    assert_eq!(lock.mods[0].sha256.as_str(), hash);
    pack.mods[0].sha256 = "0".repeat(64);
    assert!(plan(&pack).is_err());
}

#[test]
fn failed_download_leaves_previous_selection_and_undo_keeps_unrelated_settings() {
    let (registry, mut fetch) = registry(&[
        ("rails", "1.0.0", "both", ""),
        ("beams", "1.0.0", "both", ""),
    ]);
    let goal = target();
    let dir = tempfile::tempdir().unwrap();
    let base = "# Admin's comment\n[world]\nname = 'old'\n";
    fs::write(dir.path().join("server.toml"), base).unwrap();
    let folder = ServerFolder::open(dir.path()).unwrap();
    let first = selection::plan(
        &registry,
        &goal,
        input(&["rails"]),
        None,
        None,
        Refresh::None,
        |_, _| None,
    )
    .unwrap();
    let stage = folder.stage().unwrap();
    selection::stage_content(
        folder.root(),
        &stage,
        &fetch,
        &registry,
        None,
        &first,
        false,
    )
    .unwrap();
    selection::stage_records(folder.root(), &stage, None, &first.lock, base).unwrap();
    folder.commit(stage, base).unwrap();
    let old = ModLock::read(folder.root()).unwrap();
    let old_text = fs::read_to_string(folder.root().join("server.toml")).unwrap();
    let next = selection::plan(
        &registry,
        &goal,
        input(&["rails", "beams"]),
        None,
        Some(&old),
        Refresh::None,
        |_, _| None,
    )
    .unwrap();
    let beams = &registry.project("beams").unwrap().versions[0];
    fetch
        .0
        .insert(beams.file.url.clone(), vec![0; beams.file.size as usize]);
    let stage = folder.stage().unwrap();
    let error = selection::stage_content(
        folder.root(),
        &stage,
        &fetch,
        &registry,
        Some(&old),
        &next,
        false,
    )
    .unwrap_err();
    assert!(error.contains("SHA-256"), "{error}");
    drop(stage);
    assert!(!folder.root().join("sunrise-mods/beams.smod").exists());
    assert_eq!(ModLock::read(folder.root()).unwrap().input, old.input);
    assert!(
        mod_list::ModList::load(folder.root())
            .enabled
            .contains("rails")
    );
    let edited = old_text.replace("'old'", "'new'");
    fs::write(folder.root().join("server.toml"), &edited).unwrap();
    assert!(transaction::read_lease(folder.root()).is_err());
    folder.rollback(&edited).unwrap();
    assert!(!folder.root().join(crate::lock::LOCK).exists());
    assert_eq!(
        fs::read_to_string(folder.root().join("server.toml")).unwrap(),
        base.replace("'old'", "'new'")
    );
    assert!(folder.previous_lock().is_err());
}
