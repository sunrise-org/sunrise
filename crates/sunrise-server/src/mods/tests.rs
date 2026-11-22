use super::*;
use sunrise_server_launch::config::ModRequest;

const BUILD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn fixture(row: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join(LOCK), format!("format = 1\nsunrise = '{BUILD}'\ninput = {{ pack = 'pack@1.0.0', list = ['ladders@^2'], off = ['rails'] }}\n\n[[mods]]\nid = 'rails'\nversion = '1.2.3'\nsha256 = '{HASH}'\nurl = 'https://example.net/rails.smod'\n{row}\n")).unwrap();
    (dir, root)
}

#[test]
fn checks_locked_bytes_even_for_a_disabled_mod() {
    let (_dir, root) = fixture("file = 'rails.smod'\nreason = 'pack'\non = false");
    let lock = Lock::read(&root).unwrap();
    assert!(check_files(&lock, &root, &lock.input).is_err());
    let path = root.join("rails.smod");
    fs::write(&path, b"abc").unwrap();
    let config =
        Config::parse("[mods]\npack = 'pack@1'\nlist = ['ladders@^2']\noff = ['rails']").unwrap();
    let mut input = config.mods.unwrap();
    assert!(check_files(&lock, &root, &input).is_ok());
    input
        .list
        .push(ModRequest::try_from("windows".to_owned()).unwrap());
    assert!(check_files(&lock, &root, &input).is_err());
    fs::write(path, b"bad").unwrap();
    assert!(check_files(&lock, &root, &lock.input).is_err());
}

#[test]
fn refuses_nonportable_paths_at_the_lock_boundary() {
    for path in [
        "../rails.smod",
        "files/../rails.smod",
        "/rails.smod",
        "C:/rails.smod",
        r"files\rails.smod",
    ] {
        let (_dir, root) = fixture(&format!("file = '{path}'\nreason = 'requested'\non = true"));
        assert!(Lock::read(&root).is_err(), "{path}");
    }
}

#[cfg(unix)]
#[test]
fn follows_internal_paths_but_not_links_outside_the_server() {
    use std::os::unix::fs::symlink;
    let (_dir, root) = fixture("file = 'mods/rails.smod'\nreason = 'pack'\non = true");
    let external = tempfile::tempdir().unwrap();
    let lock = Lock::read(&root).unwrap();
    for target in [external.path().to_path_buf(), root.join("internal")] {
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("rails.smod"), b"abc").unwrap();
        symlink(&target, root.join("mods")).unwrap();
        assert_eq!(
            check_files(&lock, &root, &lock.input).is_ok(),
            target.starts_with(&root)
        );
        fs::remove_file(root.join("mods")).unwrap();
    }
}

#[test]
fn bounds_the_lock_read_and_does_not_accept_a_valid_prefix() {
    let (_dir, root) = fixture("file = 'rails.smod'\nreason = 'pack'\non = false");
    let path = root.join(LOCK);
    let len = fs::metadata(&path).unwrap().len();
    fs::File::options()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(&vec![b' '; (MAX_LOCK + 1 - len) as usize])
        .unwrap();
    assert!(Lock::read(&root).is_err());
}

#[test]
fn checks_native_runtime_and_requires_consent() {
    let (_dir, root) = fixture(&format!(
        "file = 'native/sources/rails.smod'\nreason = 'requested'\non = true\nnative = {{ built = '{BUILD}' }}"
    ));
    let mut lock = Lock::read(&root).unwrap();
    let native = root.join(NATIVE_DIR);
    fs::create_dir_all(native.join("sources")).unwrap();
    fs::write(native.join("sources/rails.smod"), b"abc").unwrap();
    assert!(check_files(&lock, &root, &lock.input).is_err());
    lock.input.trust_native_code = true;
    assert!(check_files(&lock, &root, &lock.input).is_ok());
    let identity =
        serde_json::json!({ "commit": BUILD, "side": "server", "rustc": "release: 1.95.0" });
    let library = serde_json::json!({ "file": "rails.so", "sha256": HASH, "identity": format!("{BUILD} rails 1.2.3 1.95.0 unwind"), "mod": "rails", "kind": "mod" });
    let record = serde_json::json!({ "sunrise": BUILD, "rust": "1.95.0", "libraries": [library] });
    let mut install = |name: &str, contents: &[u8]| {
        fs::write(native.join(name), contents).unwrap();
        let path = PackagePath::try_from(format!("{NATIVE_DIR}/{name}")).unwrap();
        lock.runtime.insert(
            path,
            sunrise_mod_install::index::sha256(contents)
                .try_into()
                .unwrap(),
        );
    };
    install(CLIENT, b"not a running process");
    install("rails.so", b"abc");
    install("native-mods.json", record.to_string().as_bytes());
    install("identity.json", identity.to_string().as_bytes());
    assert_eq!(
        check_native(&lock, &root).unwrap(),
        Some(native.join(CLIENT))
    );
    fs::write(native.join("rails.so"), b"drift").unwrap();
    assert!(
        check_native(&lock, &root)
            .unwrap_err()
            .to_string()
            .contains("different checksum")
    );
    fs::write(native.join("rails.so"), b"abc").unwrap();
    fs::write(native.join("unrecorded.so"), b"more code").unwrap();
    assert!(
        check_native(&lock, &root)
            .unwrap_err()
            .to_string()
            .contains("isn't listed")
    );
    fs::remove_file(native.join("unrecorded.so")).unwrap();
    let mut wrong = record;
    wrong["libraries"][0]["identity"] = format!("{BUILD} rails 1.2.3 1.95.1 unwind").into();
    let wrong = wrong.to_string();
    fs::write(native.join("native-mods.json"), &wrong).unwrap();
    lock.runtime.insert(
        PackagePath::try_from("native/native-mods.json".to_owned()).unwrap(),
        sunrise_mod_install::index::sha256(wrong.as_bytes())
            .try_into()
            .unwrap(),
    );
    assert!(
        check_native(&lock, &root)
            .unwrap_err()
            .to_string()
            .contains("doesn't match")
    );
}

#[test]
fn json_list_reports_the_lock_even_when_the_installed_file_is_missing() {
    let (_dir, root) =
        fixture("file = 'rails.smod'\nreason = 'required'\nrequired_by = ['ladders']\non = false");
    let mut output = Vec::new();
    write_list(&Lock::read(&root).unwrap(), &mut output, true).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let row = &json["mods"][0];
    assert_eq!(row["id"], "rails");
    assert_eq!(row["version"], "1.2.3");
    assert_eq!(row["on"], false);
    assert_eq!(row["reason"], "required");
    assert_eq!(row["required_by"][0], "ladders");
}

#[cfg(unix)]
#[test]
fn release_drift_refuses_only_a_known_selected_release() {
    use std::os::unix::fs::symlink;
    let (_server, root) = fixture("file = 'rails.smod'\nreason = 'pack'\non = false");
    let path = root.join(LOCK);
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("release = '26.3-0.1.0'\n{text}")).unwrap();
    fs::write(root.join("rails.smod"), b"abc").unwrap();
    fs::write(root.join("sunrise-mods.txt"), "mod.rails:false\n").unwrap();
    let installation = tempfile::tempdir().unwrap();
    let install = launch::Install {
        root: installation.path().into(),
    };
    for tag in ["26.3-0.1.0", "26.3-0.1.1"] {
        let release = install.release(tag);
        fs::create_dir_all(&release).unwrap();
        fs::write(release.join("sunrise-engine"), b"not executable").unwrap();
    }
    symlink(install.release("26.3-0.1.0"), install.programs()).unwrap();
    let mut config = Config {
        mods: Some(Lock::read(&root).unwrap().input),
        ..Default::default()
    };
    assert!(check_start(&root, &config, Some(&install)).is_ok());
    fs::remove_file(install.programs()).unwrap();
    symlink(install.release("26.3-0.1.1"), install.programs()).unwrap();
    assert!(check_start(&root, &config, Some(&install)).is_err());
    fs::write(root.join("custom-engine"), b"not executable").unwrap();
    config.engine.path = "custom-engine".into();
    assert!(check_start(&root, &config, Some(&install)).is_ok());
}
