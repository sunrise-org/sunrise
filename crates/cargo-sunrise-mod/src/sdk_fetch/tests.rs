use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::cell::OnceCell;
use std::sync::atomic::Ordering::Relaxed;

pub(crate) const BUILD: &str = "0123456789abcdef0123456789abcdef01234567";
const SURFACE_STUB: &str = r#"{"hash":"ab"}"#;

/// A release folder `sdk-<BUILD>` under a base folder, signed with a fresh key: the base and the
/// key.
fn release(nightly: Option<&str>) -> (tempfile::TempDir, PublicKey) {
    release_with_surface(BUILD, nightly, SURFACE_STUB)
}

pub(crate) fn release_with_surface(
    build: &str,
    nightly: Option<&str>,
    surface: &str,
) -> (tempfile::TempDir, PublicKey) {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let key: PublicKey = pair.public_key().as_ref().try_into().unwrap();
    (release_signed_by(build, nightly, surface, &pair), key)
}

/// [`release`], its `SHA256SUMS` signed by `pair`.
fn release_signed_by(
    commit: &str,
    nightly: Option<&str>,
    surface: &str,
    pair: &Ed25519KeyPair,
) -> tempfile::TempDir {
    let base = tempfile::tempdir().unwrap();
    let tag = format!("sdk-{commit}");
    let folder = base.path().join(&tag);
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(SURFACE), surface).unwrap();
    // Any archive the unpack reads stands in for the API source and the build data.
    for archive in [API_SOURCE, build_data::ARCHIVE] {
        fs::write(
            folder.join(archive),
            include_bytes!("../../../sunrise-mod-package/src/sdk/fixture.tar.xz"),
        )
        .unwrap();
    }
    let build = sdk::Build {
        commit: commit.into(),
        game: sdk::GAME.into(),
        date: "2026-11-13".into(),
        nightly: nightly.map(str::to_owned),
    };
    let mut index = Index::of_folder(&folder, build, "1.99.1".into()).unwrap();
    index.surface.notes.push(sdk::ChangeNote {
        ids: vec!["hook:example".into()],
        why: "Signed note".into(),
        migrate: None,
    });
    fs::write(folder.join(sdk::INDEX), index.to_json()).unwrap();
    let digest = |name: &str| format!("{:x}", Sha256::digest(fs::read(folder.join(name)).unwrap()));
    let sums = sunrise_release_sign::sums_file(
        &[sdk::INDEX, SURFACE, API_SOURCE].map(|name| (name.to_owned(), digest(name))),
    );
    fs::write(folder.join(sunrise_release_sign::SUMS), &sums).unwrap();
    let message = sunrise_release_sign::message(&tag, sums.as_bytes());
    use base64::Engine;
    let signature = base64::engine::general_purpose::STANDARD.encode(pair.sign(&message).as_ref());
    fs::write(
        folder.join(sunrise_release_sign::SIGNATURE),
        signature + "\n",
    )
    .unwrap();
    base
}

/// A local signed stand-in for scaffold tests that don't exercise release lookup.
pub(crate) fn offline(project: &Path, build: &str) -> Result<PinnedSdk, String> {
    let (base, key) = release_with_surface(build, None, SURFACE_STUB);
    fetch_pinned_from(
        &Base::Folder(base.path().to_owned()),
        &[key],
        project,
        build,
    )
}

pub(crate) fn pinned_fixture(dir: PathBuf, index: Index, surface_bytes: Vec<u8>) -> PinnedSdk {
    VerifiedSdk {
        index,
        surface_bytes,
    }
    .pinned(dir)
}

#[test]
fn a_signed_release_is_unpacked_once_and_reused() {
    let (base, key) = release(Some("nightly-2026-11-13"));
    let project = tempfile::tempdir().unwrap();
    let dir = project.path().join(DIR);
    let base = Base::Folder(base.path().to_owned());
    let pinned = fetch(&base, &[key], BUILD, &dir, true).unwrap();
    assert_eq!(pinned.build(), BUILD);
    assert_eq!(pinned.name, "nightly-2026-11-13 (0123456)");
    assert!(dir.join(SURFACE).is_file());
    assert!(dir.join("api/deps/a.rmeta").is_file());
    assert!(!dir.join(API_SOURCE).exists());
    // With the same trust, the folder is used as it is, with nothing read from the base.
    let gone = Base::Folder(project.path().join("nowhere"));
    assert_eq!(
        fetch(&gone, &[key], BUILD, &dir, true).unwrap().name,
        pinned.name
    );
}

#[test]
fn project_readers_keep_the_fresh_and_warm_verified_metadata() {
    let surface = format!(r#"{{"schema":{},"hash":"ab"}}"#, mod_surface::SCHEMA);
    let (base, key) = release_with_surface(BUILD, None, &surface);
    let project = tempfile::tempdir().unwrap();
    let dir = project.path().join(DIR);
    let fresh = fetch(
        &Base::Folder(base.path().to_owned()),
        &[key],
        BUILD,
        &dir,
        true,
    )
    .unwrap();
    let gone = Base::Folder(project.path().join("nowhere"));
    let warm = fetch(&gone, &[key], BUILD, &dir, true).unwrap();

    fs::write(dir.join(SURFACE), surface.replace("ab", "cd")).unwrap();
    let index_path = dir.join(sdk::INDEX);
    let index = fs::read_to_string(&index_path).unwrap();
    fs::write(&index_path, index.replace("Signed note", "Edited note")).unwrap();

    for pinned in [&fresh, &warm] {
        let chain = crate::sdk_notes::chain(pinned, None, |build| {
            panic!("unexpected older build: {build}")
        });
        assert_eq!(chain.notes.len(), 1);
        assert_eq!(chain.notes[0].note.why, "Signed note");
        assert_eq!(pinned.surface.parses.load(Relaxed), 0);
        for _ in 0..2 {
            assert_eq!(crate::uses_table::surface_of(pinned).unwrap().hash, "ab");
            assert_eq!(pinned.surface.parses.load(Relaxed), 1);
        }
    }
}

#[test]
fn native_new_and_the_index_only_pin_step_do_not_parse_the_surface() {
    let scratch = tempfile::tempdir().unwrap();
    let project = scratch.path().join("greeter");
    let (old_release, old_key) = release(None);
    let old_base = Base::Folder(old_release.path().to_owned());
    let new_parses = OnceCell::new();
    crate::new(
        &[
            project.display().to_string(),
            "--template".into(),
            "native".into(),
            "--sunrise".into(),
            BUILD.into(),
        ],
        &|dir, build| {
            let sdk = fetch_pinned_from(&old_base, &[old_key], dir, build)?;
            new_parses.set(Arc::clone(&sdk.surface.parses)).unwrap();
            Ok(sdk)
        },
    )
    .unwrap();
    assert_eq!(new_parses.into_inner().unwrap().load(Relaxed), 0);
    assert_eq!(pin(&project).unwrap(), BUILD);

    // Exercise pin's index-only fetch and manifest write, not its separate comparison report.
    let target = "89abcdef0123456789abcdef0123456789abcdef";
    let (target_release, target_key) = release_with_surface(target, None, SURFACE_STUB);
    let target_base = Base::Folder(target_release.path().to_owned());
    let pin_parses = OnceCell::new();
    crate::repin(
        &project,
        &|dir, build| {
            let sdk = fetch_build_from(&target_base, &[target_key], dir, build)?;
            pin_parses.set(Arc::clone(&sdk.surface.parses)).unwrap();
            Ok(sdk)
        },
        target,
        target,
    )
    .unwrap();
    assert_eq!(pin_parses.into_inner().unwrap().load(Relaxed), 0);
    assert_eq!(pin(&project).unwrap(), target);
}

#[test]
fn an_unreadable_verified_surface_caches_the_parse_error() {
    let (base, key) = release_with_surface(BUILD, None, r#"{"schema":0,"hash":"ab"}"#);
    let project = tempfile::tempdir().unwrap();
    let sdk = fetch_pinned_from(
        &Base::Folder(base.path().to_owned()),
        &[key],
        project.path(),
        BUILD,
    )
    .unwrap();
    assert_eq!(sdk.surface.parses.load(Relaxed), 0);
    for _ in 0..2 {
        assert!(matches!(sdk.surface.get(), Err(SurfaceError::Schema(0))));
        assert_eq!(sdk.surface.parses.load(Relaxed), 1);
    }
}

#[test]
fn the_notes_fetch_reads_only_the_signed_index_and_keeps_it() {
    let (base, key) = release(None);
    // A release whose surface is gone still gives its signed `sdk.json`: the surface is never read.
    fs::remove_file(base.path().join(format!("sdk-{BUILD}")).join(SURFACE)).unwrap();
    let project = tempfile::tempdir().unwrap();
    let base = Base::Folder(base.path().to_owned());
    let index = fetch_index_only_from(&base, &[key], project.path(), BUILD).unwrap();
    assert_eq!(index.build.commit, BUILD);
    let dir = project.path().join(CACHE).join(BUILD);
    assert!(!dir.join(SURFACE).exists());
    // Kept, it is read again with nothing fetched; changed, it is refused with nowhere to fetch.
    let gone = Base::Folder(project.path().join("nowhere"));
    assert!(fetch_index_only_from(&gone, &[key], project.path(), BUILD).is_ok());
    fs::write(dir.join(sdk::INDEX), b"{}").unwrap();
    assert!(fetch_index_only_from(&gone, &[key], project.path(), BUILD).is_err());
}

#[test]
fn a_kept_surface_changed_under_a_matching_stamp_is_fetched_again() {
    let (base, key) = release(None);
    let project = tempfile::tempdir().unwrap();
    let dir = project.path().join(DIR);
    let base = Base::Folder(base.path().to_owned());
    fetch(&base, &[key], BUILD, &dir, false).unwrap();
    let signed = fs::read(dir.join(SURFACE)).unwrap();
    fs::write(dir.join(SURFACE), b"{}").unwrap();
    // The stamp still matches, but the surface no longer verifies: with no base to fetch from
    // it is refused, and with one it is replaced by the signed copy.
    let gone = Base::Folder(project.path().join("nowhere"));
    assert!(fetch(&gone, &[key], BUILD, &dir, false).is_err());
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(SURFACE), b"{}").unwrap();
    fetch(&base, &[key], BUILD, &dir, false).unwrap();
    assert_eq!(fs::read(dir.join(SURFACE)).unwrap(), signed);
}

#[cfg(unix)]
#[test]
fn a_linked_project_cache_is_refused_and_its_destination_left_alone() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("sentinel"), b"kept").unwrap();
    std::os::unix::fs::symlink(outside.path(), project.path().join(".sunrise")).unwrap();
    assert!(fetch_pinned(project.path(), BUILD).is_err());
    assert_eq!(fs::read(outside.path().join("sentinel")).unwrap(), b"kept");
}

/// Every folder (no contents) and file under `dir` with its contents, to show a refused read
/// wrote nothing.
fn listing(dir: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    fn walk(dir: &Path, found: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                found.push((path.clone(), None));
                walk(&path, found);
            } else {
                found.push((path.clone(), Some(fs::read(&path).unwrap())));
            }
        }
    }
    let mut found = Vec::new();
    walk(dir, &mut found);
    found.sort();
    found
}

#[cfg(unix)]
#[test]
fn a_warm_signed_cache_behind_a_linked_build_folder_is_refused_and_left_alone() {
    let (base, key) = release(None);
    let base = Base::Folder(base.path().to_owned());
    let outside = tempfile::tempdir().unwrap();
    let referent = outside.path().join("sdk");
    // Signed, stamped and complete: only the link makes it unacceptable.
    fetch(&base, &[key], BUILD, &referent, false).unwrap();
    let before = listing(&referent);
    let project = tempfile::tempdir().unwrap();
    let cache = project.path().join(CACHE);
    fs::create_dir_all(&cache).unwrap();
    std::os::unix::fs::symlink(&referent, cache.join(BUILD)).unwrap();
    let gone = Base::Folder(project.path().join("nowhere"));
    let error = fetch_build_from(&gone, &[key], project.path(), BUILD).unwrap_err();
    assert!(error.contains("is not a real directory"), "{error}");
    assert_eq!(listing(&referent), before);
}

#[cfg(unix)]
#[test]
fn warm_build_data_behind_a_linked_folder_is_refused_and_left_alone() {
    let (base, key) = release(None);
    let base = Base::Folder(base.path().to_owned());
    let project = tempfile::tempdir().unwrap();
    let into = build_data_from(&base, &[key], project.path(), BUILD).unwrap();
    // The build's stamp still matches, but `build-data` now leads to a folder nothing verified.
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("internals.json"), b"unverified").unwrap();
    fs::remove_dir_all(&into).unwrap();
    std::os::unix::fs::symlink(outside.path(), &into).unwrap();
    let before = listing(outside.path());
    let gone = Base::Folder(project.path().join("nowhere"));
    let error = build_data_from(&gone, &[key], project.path(), BUILD).unwrap_err();
    assert!(error.contains("is not a real directory"), "{error}");
    assert_eq!(listing(outside.path()), before);
}

#[test]
fn a_stamp_longer_than_the_expected_one_is_a_miss_and_is_repaired() {
    let (base, key) = release(None);
    let base = Base::Folder(base.path().to_owned());
    let project = tempfile::tempdir().unwrap();
    let dir = project.path().join(DIR);
    fetch(&base, &[key], BUILD, &dir, false).unwrap();
    let stamp = CacheStamp::new(BUILD, &[key]);
    assert!(stamp.matches(&dir));
    // One byte past the exact stamp is beyond the read's bound.
    fs::write(dir.join(STAMP), format!("{}\n", stamp.as_str())).unwrap();
    assert!(!stamp.matches(&dir));
    fetch(&base, &[key], BUILD, &dir, false).unwrap();
    assert!(stamp.matches(&dir));
}

#[cfg(unix)]
#[test]
fn a_stamp_behind_a_link_is_a_miss_and_its_referent_is_left_alone() {
    let (base, key) = release(None);
    let base = Base::Folder(base.path().to_owned());
    let project = tempfile::tempdir().unwrap();
    let dir = project.path().join(DIR);
    fetch(&base, &[key], BUILD, &dir, false).unwrap();
    let stamp = CacheStamp::new(BUILD, &[key]);
    // The linked file holds exactly the right stamp, beside signed files that all verify.
    let outside = tempfile::tempdir().unwrap();
    let referent = outside.path().join("stamp");
    fs::rename(dir.join(STAMP), &referent).unwrap();
    std::os::unix::fs::symlink(&referent, dir.join(STAMP)).unwrap();
    assert!(!stamp.matches(&dir));
    let gone = Base::Folder(project.path().join("nowhere"));
    assert!(fetch(&gone, &[key], BUILD, &dir, false).is_err());
    assert_eq!(fs::read(&referent).unwrap(), stamp.as_str().as_bytes());
    // With a base it is fetched again into the folder itself, not through the link.
    fetch(&base, &[key], BUILD, &dir, false).unwrap();
    assert!(stamp.matches(&dir));
    assert_eq!(fs::read(&referent).unwrap(), stamp.as_str().as_bytes());
}

#[test]
fn a_prepared_sdk_is_read_without_fetching_or_writing() {
    let surface = format!(r#"{{"schema":{},"hash":"ab"}}"#, mod_surface::SCHEMA);
    let (base, key) = release_with_surface(BUILD, Some("nightly-2026-11-13"), &surface);
    let project = tempfile::tempdir().unwrap();
    let base = Base::Folder(base.path().to_owned());
    let fetched = fetch(&base, &[key], BUILD, &project.path().join(DIR), true).unwrap();
    let before = listing(project.path());
    let prepared = prepared_sdk_from(&[key], project.path(), BUILD).unwrap();
    assert_eq!(prepared.build(), BUILD);
    assert_eq!(prepared.name, fetched.name);
    assert_eq!(prepared.dir, project.path().join(DIR));
    assert_eq!(crate::uses_table::surface_of(&prepared).unwrap().hash, "ab");
    assert_eq!(listing(project.path()), before);
}

#[test]
fn a_missing_stale_or_tampered_sdk_is_refused_to_prepare_and_nothing_changes() {
    let (base, key) = release(None);
    let (_, other) = release(None);
    let project = tempfile::tempdir().unwrap();
    // Nothing prepared: not even `.sunrise` is created.
    let error = prepared_sdk_from(&[key], project.path(), BUILD).unwrap_err();
    assert!(error.contains("cargo sunrise-mod check"), "{error}");
    assert!(!project.path().join(".sunrise").exists());
    let base = Base::Folder(base.path().to_owned());
    fetch(&base, &[key], BUILD, &project.path().join(DIR), true).unwrap();
    // Prepared under other trust, or for another build.
    let before = listing(project.path());
    let error = prepared_sdk_from(&[other], project.path(), BUILD).unwrap_err();
    assert!(error.contains("cargo sunrise-mod check"), "{error}");
    let error = prepared_sdk_from(&[key], project.path(), &"1".repeat(40)).unwrap_err();
    assert!(error.contains("cargo sunrise-mod check"), "{error}");
    assert_eq!(listing(project.path()), before);
    // Stamped, but the surface is no longer the signed one: the diagnostic names it, and the
    // folder is not repaired as `check` would.
    let surface = project.path().join(DIR).join(SURFACE);
    fs::write(&surface, b"{}").unwrap();
    let before = listing(project.path());
    let error = prepared_sdk_from(&[key], project.path(), BUILD).unwrap_err();
    assert!(
        error.contains("does not verify against its signed sums"),
        "{error}"
    );
    assert!(error.contains("cargo sunrise-mod check"), "{error}");
    assert_eq!(listing(project.path()), before);
    assert_eq!(fs::read(&surface).unwrap(), b"{}");
}

#[cfg(unix)]
#[test]
fn a_linked_prepared_sdk_is_refused_to_read_and_its_destination_left_alone() {
    let (base, key) = release(None);
    let base = Base::Folder(base.path().to_owned());
    let outside = tempfile::tempdir().unwrap();
    fetch(&base, &[key], BUILD, &outside.path().join(DIR), true).unwrap();
    let before = listing(outside.path());
    // The folder itself, and the `.sunrise` above it, each lead to a prepared, verified SDK.
    let linked_folder = tempfile::tempdir().unwrap();
    fs::create_dir(linked_folder.path().join(".sunrise")).unwrap();
    std::os::unix::fs::symlink(outside.path().join(DIR), linked_folder.path().join(DIR)).unwrap();
    let linked_parent = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(
        outside.path().join(".sunrise"),
        linked_parent.path().join(".sunrise"),
    )
    .unwrap();
    for project in [&linked_folder, &linked_parent] {
        let error = prepared_sdk_from(&[key], project.path(), BUILD).unwrap_err();
        assert!(error.contains("is not a real directory"), "{error}");
    }
    assert_eq!(listing(outside.path()), before);
}

#[test]
fn a_cached_local_sdk_is_refused_under_release_only_trust() {
    let (base, key) = release(None);
    let base = Base::Folder(base.path().to_owned());
    let project = tempfile::tempdir().unwrap();
    let release_keys = sunrise_release_sign::trusted_keys();
    let mut local_keys = release_keys.clone();
    local_keys.push(key);
    for api in [true, false] {
        // Same verifier for the project's API source and the per-user native SDK metadata.
        let dir = project.path().join(if api { DIR } else { "sdk" });
        fetch(&base, &local_keys, BUILD, &dir, api).unwrap();
        assert!(fetch(&base, &release_keys, BUILD, &dir, api).is_err());
        assert!(!dir.join(STAMP).exists());
        assert!(!dir.join(sdk::INDEX).exists());
        assert!(!dir.join("api").exists());
    }
}

#[test]
fn a_release_not_signed_by_a_trusted_key_or_not_matching_its_sums_is_refused() {
    let (base, key) = release(None);
    let project = tempfile::tempdir().unwrap();
    let dir = project.path().join(DIR);
    let folder = base.path().join(format!("sdk-{BUILD}"));
    let base = Base::Folder(base.path().to_owned());
    let (_, other) = release(None);
    let error = fetch(&base, &[other], BUILD, &dir, true).unwrap_err();
    assert!(error.contains("signature doesn't match"), "{error}");
    assert!(!dir.join(STAMP).exists());
    // A changed sdk.json is not the one the signed sums list.
    let index = fs::read_to_string(folder.join(sdk::INDEX)).unwrap();
    fs::write(
        folder.join(sdk::INDEX),
        index.replacen("\"ab\"", "\"cd\"", 1),
    )
    .unwrap();
    let error = fetch(&base, &[key], BUILD, &dir, true).unwrap_err();
    assert!(
        error.contains("is not the one its signed SHA256SUMS list"),
        "{error}"
    );
    fs::write(folder.join(sdk::INDEX), index).unwrap();
    // An archive that is not the one sdk.json lists.
    fs::write(folder.join(API_SOURCE), b"not the archive").unwrap();
    let error = fetch(&base, &[key], BUILD, &dir, true).unwrap_err();
    assert!(error.starts_with(&format!("sdk-{BUILD}: ")), "{error}");
    assert!(!dir.join(STAMP).exists());
    // Another build's release under this build's name.
    let other = "89abcdef0123456789abcdef0123456789abcdef";
    fs::rename(&folder, base_folder(&base).join(format!("sdk-{other}"))).unwrap();
    assert!(fetch(&base, &[key], other, &dir, false).is_err());
}

#[test]
fn build_data_comes_from_the_signed_sdk_once_and_only_from_it() {
    let (base, key) = release(None);
    let folder = base.path().join(format!("sdk-{BUILD}"));
    let base = Base::Folder(base.path().to_owned());
    let project = tempfile::tempdir().unwrap();
    // A build with no SDK release is refused by name.
    let other = "89abcdef0123456789abcdef0123456789abcdef";
    let error = build_data_from(&base, &[key], project.path(), other).unwrap_err();
    assert!(
        error.starts_with(&format!(
            "game build {other} has no signed SDK (release sdk-{other}): "
        )),
        "{error}"
    );
    // An archive that is not the one sdk.json lists leaves no build data.
    let archive = fs::read(folder.join(build_data::ARCHIVE)).unwrap();
    fs::write(folder.join(build_data::ARCHIVE), b"not the archive").unwrap();
    let error = build_data_from(&base, &[key], project.path(), BUILD).unwrap_err();
    assert!(error.starts_with(&format!("sdk-{BUILD}: ")), "{error}");
    let into = project.path().join(CACHE).join(BUILD).join(BUILD_DATA);
    assert!(!into.exists());
    fs::write(folder.join(build_data::ARCHIVE), archive).unwrap();
    assert_eq!(
        build_data_from(&base, &[key], project.path(), BUILD).unwrap(),
        into
    );
    assert!(into.join("deps/a.rmeta").is_file());
    assert!(!into.with_file_name(build_data::ARCHIVE).exists());
    // Unpacked once: later reads need no release.
    let gone = Base::Folder(project.path().join("nowhere"));
    assert_eq!(
        build_data_from(&gone, &[key], project.path(), BUILD).unwrap(),
        into
    );
}

#[test]
fn cached_local_build_data_is_refused_under_release_only_trust() {
    let (base, key) = release(None);
    let base = Base::Folder(base.path().to_owned());
    let project = tempfile::tempdir().unwrap();
    let release_keys = sunrise_release_sign::trusted_keys();
    let mut local_keys = release_keys.clone();
    local_keys.push(key);
    let into = build_data_from(&base, &local_keys, project.path(), BUILD).unwrap();
    assert!(into.is_dir());
    assert!(build_data_from(&base, &release_keys, project.path(), BUILD).is_err());
    assert!(!into.exists());
}

fn base_folder(base: &Base) -> &Path {
    match base {
        Base::Folder(folder) => folder,
        Base::Url(_) => unreachable!("a folder"),
    }
}

/// `<base>/<tag>/` holding `index` as its `sdk.json`, with SHA256SUMS listing `listed` (the index's
/// own bytes when None) signed by `pair` for `signed_for` into `signature`.
fn tag_release(
    base: &Path,
    tag: &str,
    index: &str,
    listed: Option<&str>,
    signed_for: &str,
    signature: &str,
    pair: &Ed25519KeyPair,
) {
    let folder = base.join(tag);
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(sdk::INDEX), index).unwrap();
    let sums = sunrise_release_sign::sums_file(&[(
        sdk::INDEX.to_owned(),
        sunrise_release_sign::sha256_hex(listed.unwrap_or(index).as_bytes()),
    )]);
    fs::write(folder.join(sunrise_release_sign::SUMS), &sums).unwrap();
    let message = sunrise_release_sign::message(signed_for, sums.as_bytes());
    use base64::Engine;
    let signed = base64::engine::general_purpose::STANDARD.encode(pair.sign(&message).as_ref());
    fs::write(folder.join(signature), signed + "\n").unwrap();
}

fn index_of(commit: &str) -> String {
    format!(r#"{{"schema":1,"build":{{"commit":"{commit}"}}}}"#)
}

#[test]
fn a_build_is_named_by_sha_short_sha_or_a_release_tag() {
    let (base, _) = release(None);
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let keys: [PublicKey; 1] = [pair.public_key().as_ref().try_into().unwrap()];
    let index = index_of(BUILD);
    // A Nightly name is a pointer, as a mirror holds it; a Stable tag and a Dev build are signed
    // for themselves.
    fs::create_dir_all(base.path().join("nightly-2026-11-13")).unwrap();
    fs::write(
        base.path().join("nightly-2026-11-13").join(sdk::INDEX),
        &index,
    )
    .unwrap();
    let stable = "26.3-0.1.0";
    let dev = "dev-2026-11-21-0123456";
    let v2 = sunrise_release_sign::SIGNATURE_V2;
    tag_release(
        base.path(),
        stable,
        &index,
        None,
        stable,
        sunrise_release_sign::SIGNATURE,
        &pair,
    );
    tag_release(base.path(), dev, &index, None, dev, v2, &pair);
    let base = Base::Folder(base.path().to_owned());
    assert_eq!(resolve(&base, &keys, BUILD).unwrap(), BUILD);
    assert_eq!(resolve(&base, &keys, "0123456").unwrap(), BUILD);
    for tag in ["nightly-2026-11-13", stable, dev] {
        assert_eq!(resolve(&base, &keys, tag).unwrap(), BUILD, "{tag}");
    }
    let error = resolve(&base, &keys, "nightly").unwrap_err();
    assert!(error.starts_with("Release nightly has no SDK"), "{error}");
    assert!(
        resolve(&base, &keys, "89abcde")
            .unwrap_err()
            .contains("no SDK release")
    );
    assert!(
        resolve(&base, &keys, "main")
            .unwrap_err()
            .contains("is not a build")
    );
}

#[test]
fn a_release_tag_resolves_only_through_its_own_signed_index() {
    let base = tempfile::tempdir().unwrap();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let keys: [PublicKey; 1] = [pair.public_key().as_ref().try_into().unwrap()];
    let older = "89abcdef0123456789abcdef0123456789abcdef";
    let v1 = sunrise_release_sign::SIGNATURE;
    let v2 = sunrise_release_sign::SIGNATURE_V2;
    // Another build's index served under the tag, the tag's sums listing the real one.
    tag_release(
        base.path(),
        "26.3-0.1.0",
        &index_of(older),
        Some(&index_of(BUILD)),
        "26.3-0.1.0",
        v1,
        &pair,
    );
    // Sums signed for another release.
    tag_release(
        base.path(),
        "26.3-0.1.1",
        &index_of(BUILD),
        None,
        "26.3-0.1.0",
        v1,
        &pair,
    );
    // A Dev tag whose signed index names a build that its short sha does not start.
    let dev = "dev-2026-11-21-0123456";
    tag_release(base.path(), dev, &index_of(older), None, dev, v2, &pair);
    // A Dev build signed only for its channel.
    let unsigned = "dev-2026-11-22-0123456";
    tag_release(
        base.path(),
        unsigned,
        &index_of(BUILD),
        None,
        "dev",
        v1,
        &pair,
    );
    // Sums signed for the tag that are not a sums list.
    let malformed = "26.3-0.1.2";
    tag_release(
        base.path(),
        malformed,
        &index_of(BUILD),
        None,
        malformed,
        v1,
        &pair,
    );
    let folder = base.path().join(malformed);
    let sums = b"not a checksum list\n";
    fs::write(folder.join(sunrise_release_sign::SUMS), sums).unwrap();
    let message = sunrise_release_sign::message(malformed, sums);
    use base64::Engine;
    let signed = base64::engine::general_purpose::STANDARD.encode(pair.sign(&message).as_ref());
    fs::write(folder.join(v1), signed + "\n").unwrap();
    let base = Base::Folder(base.path().to_owned());
    for tag in ["26.3-0.1.0", "26.3-0.1.1", dev, unsigned, malformed] {
        let error = resolve(&base, &keys, tag).unwrap_err();
        assert!(
            error.starts_with(&format!("Release {tag}")),
            "{tag}: {error}"
        );
    }
}

#[test]
fn a_release_tag_signed_only_by_a_local_sdk_key_is_refused() {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let local: PublicKey = pair.public_key().as_ref().try_into().unwrap();
    let root = release_signed_by(BUILD, None, SURFACE_STUB, &pair);
    let index = index_of(BUILD);
    let (stable, dev) = ("26.3-0.1.0", "dev-2026-11-21-0123456");
    let (v1, v2) = (
        sunrise_release_sign::SIGNATURE,
        sunrise_release_sign::SIGNATURE_V2,
    );
    tag_release(root.path(), stable, &index, None, stable, v1, &pair);
    tag_release(root.path(), dev, &index, None, dev, v2, &pair);
    let base = Base::Folder(root.path().to_owned());
    let mut sdk_keys = release_keys();
    sdk_keys.push(local);
    // The local key is trusted for the SDK, and the tags are otherwise valid.
    let project = tempfile::tempdir().unwrap();
    fetch(&base, &sdk_keys, BUILD, &project.path().join(DIR), true).unwrap();
    for tag in [stable, dev] {
        assert_eq!(resolve(&base, &sdk_keys, tag).unwrap(), BUILD, "{tag}");
        let error = resolve(&base, &release_keys(), tag).unwrap_err();
        assert!(error.contains("signature doesn't match"), "{tag}: {error}");
    }
}

#[test]
fn a_nightly_name_is_only_nightly_or_a_dated_one() {
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("releases");
    // A name that read through the base would reach this decoy outside it.
    fs::create_dir_all(base.join("nightly-")).unwrap();
    let decoy = root.path().join("decoy");
    fs::create_dir_all(&decoy).unwrap();
    fs::write(
        decoy.join(sdk::INDEX),
        format!(r#"{{"schema":1,"build":{{"commit":"{BUILD}"}}}}"#),
    )
    .unwrap();
    let base = Base::Folder(base);
    for text in [
        "nightly-/../../decoy",
        "nightly-2026-11-1x",
        "nightly-2026/11/13",
        "nightly-2026-11-130",
        "nightly-",
        "26.3-0.1.0-rc.1",
        "26.3-0.1",
        "026.3-0.1.0",
        "26.3.1.2-0.1.0",
        "dev-2026-11-21",
        "dev-2026-11-21-01234",
        "dev-2026-11-21-/../../decoy",
    ] {
        let error = resolve(&base, &[], text).unwrap_err();
        assert!(error.contains("is not a build"), "{text}: {error}");
    }
}
