use super::*;

const CURRENT: &str = r#"{"commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","date":"2026-11-21T16:00:00Z","supports":{"min":1,"max":2}}"#;
const NEXT: &str = r#"{"commit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","date":"2026-11-21T16:00:01Z","supports":{"min":1,"max":2}}"#;
const ARCHIVE: &str = "server-tool-linux-x86_64.tar.gz";

#[test]
fn another_commit_on_the_same_day_can_be_newer() {
    let current: Build = serde_json::from_str(CURRENT).unwrap();
    let next: Build = serde_json::from_str(NEXT).unwrap();
    assert!(next.newer_than(&current).unwrap());
    assert!(!current.newer_than(&current).unwrap());
    let relabeled: Build = serde_json::from_str(&CURRENT.replace("16:00:00", "16:00:01")).unwrap();
    assert!(!relabeled.newer_than(&current).unwrap());
}

#[test]
fn refuses_a_rolling_release_with_an_older_build() {
    let current: Build = serde_json::from_str(NEXT).unwrap();
    let older: Build = serde_json::from_str(CURRENT).unwrap();
    assert!(older.newer_than(&current).is_err());
}

#[test]
fn each_architecture_selects_its_own_archive() {
    assert_eq!(
        platform_archive("windows", "aarch64").unwrap(),
        "server-tool-windows-arm64.zip"
    );
    assert_eq!(
        platform_archive("windows", "x86_64").unwrap(),
        "server-tool-windows-x64.zip"
    );
    let error = platform_archive("linux", "aarch64").unwrap_err();
    assert!(error.contains("linux aarch64"));
}

#[test]
fn refuses_a_manager_that_drops_the_installed_contract() {
    let next: Build = serde_json::from_str(NEXT).unwrap();
    assert!(next.check_game("26.3-0.1.0", 1).is_ok());
    assert!(next.check_game("26.3-0.1.1", 2).is_ok());
    let refusal = next.check_game("26.3-0.2.0", 3).unwrap_err();
    assert!(refusal.contains("26.3-0.2.0"));
    assert!(refusal.contains("contract 3"));
    assert!(refusal.contains("1–2"));
}

#[test]
fn authenticates_the_release_tag_and_compatibility_metadata() {
    let (key, _) = sign::SigningKey::generate().unwrap();
    let sums = sign::sums_file(&[
        (MANIFEST.into(), sign::sha256_hex(NEXT.as_bytes())),
        (ARCHIVE.into(), sign::sha256_hex(b"archive")),
    ]);
    let fixture = |signed_for: &str, manifest: &str| {
        let release = GithubRelease {
            tag_name: TAG.into(),
            assets: [sign::SUMS, sign::SIGNATURE, MANIFEST, ARCHIVE]
                .into_iter()
                .map(|name| Asset {
                    name: name.into(),
                    browser_download_url: name.into(),
                })
                .collect(),
        };
        signed_release(
            release,
            ARCHIVE,
            |file| {
                Ok(match file {
                    sign::SUMS => sums.as_bytes().to_vec(),
                    sign::SIGNATURE => key.sign(signed_for, sums.as_bytes()).into_bytes(),
                    MANIFEST => manifest.as_bytes().to_vec(),
                    _ => panic!("unexpected fetch: {file}"),
                })
            },
            &[key.public()],
        )
    };
    let valid = fixture(TAG, NEXT).unwrap();
    assert_eq!(valid.build.commit, "b".repeat(40));
    assert_eq!(valid.sha256, sign::sha256_hex(b"archive"));
    assert!(fixture(sign::NIGHTLY, NEXT).is_err());
    assert!(fixture(TAG, CURRENT).is_err());
}

#[test]
fn refuses_an_archive_with_the_wrong_checksum() {
    let expected = sign::sha256_hex(b"good");
    assert!(update::copy_verified(ARCHIVE, &expected, b"bad".as_slice(), io::sink()).is_err());
}

#[test]
fn replacing_and_rolling_back_exchange_the_manager_only() {
    let folder = tempfile::tempdir().unwrap();
    let exe = folder.path().join(update::exe("sunrise-server"));
    let previous = sibling(&exe, ".previous");
    let game = folder.path().join(update::exe("sunrise-engine"));
    fs::write(&exe, b"old manager").unwrap();
    fs::write(&game, b"game").unwrap();

    let staged = tempfile::tempdir_in(folder.path()).unwrap();
    let incoming = staged.path().join("incoming");
    fs::write(&incoming, b"new manager").unwrap();
    replace(&exe, &incoming, staged.path()).unwrap();
    assert_eq!(fs::read(&exe).unwrap(), b"new manager");
    assert_eq!(fs::read(&previous).unwrap(), b"old manager");

    let staged = tempfile::tempdir_in(folder.path()).unwrap();
    rollback_to_previous(&exe, staged.path()).unwrap();
    assert_eq!(fs::read(&exe).unwrap(), b"old manager");
    assert_eq!(fs::read(&previous).unwrap(), b"new manager");
    assert_eq!(fs::read(&game).unwrap(), b"game");
}

#[test]
fn a_failed_swap_keeps_the_manager_and_its_previous_copy() {
    let folder = tempfile::tempdir().unwrap();
    let exe = folder.path().join(update::exe("sunrise-server"));
    let previous = sibling(&exe, ".previous");
    fs::write(&exe, b"old manager").unwrap();
    fs::write(&previous, b"previous manager").unwrap();
    let staged = tempfile::tempdir_in(folder.path()).unwrap();
    let missing = staged.path().join("missing");
    assert!(replace(&exe, &missing, staged.path()).is_err());
    assert_eq!(fs::read(&exe).unwrap(), b"old manager");
    assert_eq!(fs::read(&previous).unwrap(), b"previous manager");
}

#[cfg(unix)]
#[test]
fn a_tool_tarball_stages_an_executable_but_no_other_paths() {
    use std::os::unix::fs::PermissionsExt;
    let folder = tempfile::tempdir().unwrap();
    let archive = folder.path().join(ARCHIVE);
    let incoming = folder.path().join("incoming");
    let make_archive = |name| {
        let encoder = flate2::write::GzEncoder::new(
            File::create(&archive).unwrap(),
            flate2::Compression::fast(),
        );
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o644);
        builder.append_data(&mut header, name, &b"exe"[..]).unwrap();
        builder.into_inner().unwrap().finish().unwrap();
    };
    make_archive("nested/sunrise-server");
    assert!(unpack(&archive, &incoming).is_err());
    assert!(!incoming.exists());
    make_archive("sunrise-server");
    unpack(&archive, &incoming).unwrap();
    assert_eq!(fs::read(&incoming).unwrap(), b"exe");
    assert_ne!(
        fs::metadata(&incoming).unwrap().permissions().mode() & 0o100,
        0
    );
}
