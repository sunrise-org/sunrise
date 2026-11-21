use super::{RELEASE_PIN, RELEASE_TAG, Registry};
use mod_surface::uses::{Entry, USES_FILE, USES_SCHEMA, UsesTable};
use serde_json::{Value, json};
use std::{fs, path::Path};
use sunrise_mod_package::{build_data, index::sha256, manifest::API, sdk};

const DAY: &str = "2026-11-20";
const TARGET: &str = "sunrise_client::game::example";

pub(super) fn sdk(
    root: &Path,
    commit: &str,
    surface_schema: u32,
    api: &str,
    content_api: &str,
    functions: Value,
) {
    let dir = root.join(commit);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("surface.json"),
        json!({
            "schema": surface_schema, "hash": api,
            "functions": [{"id": "fn:example", "hash": api, "signature": "fn example()"}]
        })
        .to_string(),
    )
    .unwrap();
    let build = sdk::Build {
        commit: commit.into(),
        game: "26.3".into(),
        date: DAY.into(),
        nightly: None,
    };
    let mut index = sdk::Index::of_folder(&dir, build, "1.99.1".into()).unwrap();
    index.content_api = content_api.into();
    fs::write(dir.join(sdk::INDEX), index.to_json()).unwrap();
    let data = dir.join("build-data");
    fs::create_dir(&data).unwrap();
    let build = build_data::Build {
        commit: commit.into(),
    };
    for (name, body) in [
        (
            build_data::FUNCTIONS,
            json!({"crates": {}, "functions": functions, "refused": {}}),
        ),
        (build_data::INTERNALS, json!({"internals": []})),
        (build_data::LOOPS, json!({"loops": []})),
    ] {
        fs::write(
            data.join(name),
            serde_json::to_vec(&build_data::Document::new(&build, body)).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn registry_judges_earlier_releases_on_the_line_and_the_current_nightly() {
    let r = Registry::new();
    let root = r.0.path();
    let made_for = "26.3-0.2.0";
    let signature_break = "26.3-0.2.1";
    let api_break = "26.3-0.2.2";
    let content_break = "26.3-0.2.3";
    let older = "a".repeat(40);
    let other_line = "b".repeat(40);
    let nightly = "c".repeat(40);
    let changed_signature = "d".repeat(40);
    let changed_api = "e".repeat(40);
    let changed_content_api = "f".repeat(40);
    let releases = root.join("releases.json");
    fs::write(
        &releases,
        json!([
            {"tag": RELEASE_TAG, "commit": older}, {"tag": made_for, "commit": RELEASE_PIN},
            {"tag": signature_break, "commit": changed_signature},
            {"tag": api_break, "commit": changed_api},
            {"tag": content_break, "commit": changed_content_api},
            {"tag": "26.3-1.0.0", "commit": other_line}
        ])
        .to_string(),
    )
    .unwrap();
    let nightly_file = root.join("nightly-build.json");
    let nightly_build = json!({"date": DAY, "commit": nightly});
    fs::write(&nightly_file, nightly_build.to_string()).unwrap();
    let data = root.join("sdk");
    let original = "pub(crate) fn example() {}";
    for (commit, signature, span, api, content_api) in [
        (&*older, "sig", original, "api", API),
        (RELEASE_PIN, "sig", original, "api", API),
        (&*nightly, "sig", "fn example() {}", "api", API),
        (
            &*changed_signature,
            "another-sig",
            "fn example(_: i32) {}",
            "api",
            API,
        ),
        (&*changed_api, "sig", original, "different-api", API),
        (&*changed_content_api, "sig", original, "api", "99.0"),
    ] {
        sdk(
            &data,
            commit,
            mod_surface::SCHEMA,
            api,
            content_api,
            json!({TARGET: {
                "module": "sunrise_client::game", "file": "fixture.rs",
                "span": [0, span.len()], "signature_hash": signature, "sha256": sha256(span.as_bytes())
            }}),
        );
    }

    let rev = sunrise_mod_package::mixins_tool::REV;
    let manifest = format!(
        "id='patched'\nname='Patched'\nversion='1.0.0'\napi='0.2'\n[native]\ngame='26.3'\nsunrise='{RELEASE_PIN}'\nmixins='{rev}'\n\
        [[native.mixin]]\nid='patch'\ntarget='{TARGET}'\nkind='HEAD'\nhelper='src/helper.rs'\n"
    );
    let game_item = format!("game:fn:{TARGET}");
    let recorded = Entry::plain(
        game_item.clone(),
        format!("sig:{}", sha256(original.as_bytes())),
    );
    let uses = UsesTable {
        schema: USES_SCHEMA,
        made_for: RELEASE_PIN.into(),
        entries: vec![
            Entry::plain("fn:example".into(), "api".into()),
            recorded.clone(),
        ],
    }
    .to_json();
    r.add_package(
        "patched",
        "1.0.0",
        "26.3",
        "native-mod",
        &[
            (sunrise_mod_package::manifest::MANIFEST, &manifest),
            ("Cargo.toml", "[package]\nname='patched'\nversion='1.0.0'\n"),
            ("src/lib.rs", ""),
            ("src/helper.rs", ""),
            (USES_FILE, &uses),
        ],
    );
    r.pin_source("patched", made_for);
    r.add("legacy", "1.0.0", "26.3", "");
    r.add_package(
        "unlisted",
        "1.0.0",
        "26.3",
        "native-mod",
        &[
            (sunrise_mod_package::manifest::MANIFEST, &format!(
                "id='unlisted'\nname='Unlisted'\nversion='1.0.0'\napi='0.2'\n[native]\ngame='26.3'\nsunrise='{RELEASE_PIN}'\n"
            )),
            ("Cargo.toml", "[package]\nname='unlisted'\nversion='1.0.0'\n"),
            ("src/lib.rs", ""),
        ],
    );
    r.pin_source("unlisted", made_for);
    let deferred = UsesTable {
        schema: 2,
        made_for: RELEASE_PIN.into(),
        entries: vec![
            Entry::plain("type:Vanished.0".into(), "old-field".into()),
            recorded,
        ],
    }
    .to_json();
    r.add_package(
        "deferred",
        "1.0.0",
        "26.3",
        "native-mod",
        &[
            (
                sunrise_mod_package::manifest::MANIFEST,
                &manifest.replace("id='patched'", "id='deferred'"),
            ),
            (
                "Cargo.toml",
                "[package]\nname='deferred'\nversion='1.0.0'\n",
            ),
            ("src/lib.rs", ""),
            ("src/helper.rs", ""),
            (USES_FILE, &deferred),
        ],
    );
    r.pin_source("deferred", made_for);

    let placeholder = r.add("glow", "1.0.0", "26.3", "");
    let shader = include_bytes!("../../../../sunrise-mod-package/tests/index/glow-1.0.0.sshader");
    let hash = sha256(shader);
    fs::write(root.join("files").join(&hash), shader).unwrap();
    r.edit("glow", |t| {
        t.replace("kind='mod'", "kind='shader'")
            .replace("glow-1.0.0.smod", "glow-1.0.0.sshader")
            .replace("categories=['utility']", "categories=['bloom']")
            .replace(&placeholder, &hash)
    });

    let index = r
        .build(&[
            "--releases",
            releases.to_str().unwrap(),
            "--nightly-build",
            nightly_file.to_str().unwrap(),
            "--sdk-data",
            data.to_str().unwrap(),
        ])
        .unwrap();
    assert_eq!(
        serde_json::to_value(index.nightly_build.as_ref().unwrap()).unwrap(),
        nightly_build
    );
    let version = &index.project("patched").unwrap().versions[0];
    assert_eq!(
        serde_json::to_value(&version.verdicts).unwrap(),
        json!({
            (RELEASE_TAG): {"verdict": "fits", "targets": []},
            (made_for): {"verdict": "made-for", "targets": []},
            (signature_break): {"verdict": "breaks", "targets": [], "changed": [game_item]},
            (api_break): {"verdict": "breaks", "targets": [], "changed": ["fn:example"]},
            (content_break): {"verdict": "breaks", "targets": [], "changed": ["content-api"]},
            "nightly": {"verdict": "breaks", "targets": [], "changed": [game_item]}
        })
    );
    let unlisted = &index.project("unlisted").unwrap().versions[0].verdicts;
    assert_eq!(
        serde_json::to_value(&unlisted[RELEASE_TAG]).unwrap(),
        json!({"verdict": "exact", "targets": [], "reason": "no uses list"})
    );
    assert_eq!(
        serde_json::to_value(&unlisted[content_break]).unwrap(),
        json!({"verdict": "breaks", "targets": [], "changed": ["content-api"]})
    );
    let deferred = &index.project("deferred").unwrap().versions[0].verdicts;
    let old = serde_json::to_value(&deferred[RELEASE_TAG]).unwrap();
    assert_eq!(old["verdict"], "exact");
    assert!(old["reason"].as_str().unwrap().contains("type:Vanished.0"));
    assert_eq!(
        serde_json::to_value(&deferred["nightly"]).unwrap(),
        json!({"verdict": "breaks", "targets": [], "changed": [game_item]})
    );
    for id in ["legacy", "glow"] {
        assert!(index.project(id).unwrap().versions[0].verdicts.is_empty());
    }
}

#[test]
fn versions_without_sunrise_do_not_need_sdk_data() {
    let r = Registry::new();
    r.add("legacy", "1.0.0", "26.3", "");
    let root = r.0.path();
    let releases = root.join("releases.json");
    fs::write(
        &releases,
        json!([{"tag": RELEASE_TAG, "commit": RELEASE_PIN}]).to_string(),
    )
    .unwrap();
    let nightly = root.join("nightly.json");
    fs::write(
        &nightly,
        json!({"date": DAY, "commit": "b".repeat(40)}).to_string(),
    )
    .unwrap();
    let index = r
        .build(&[
            "--releases",
            releases.to_str().unwrap(),
            "--nightly-build",
            nightly.to_str().unwrap(),
        ])
        .unwrap();
    assert!(
        index.project("legacy").unwrap().versions[0]
            .verdicts
            .is_empty()
    );
    assert!(index.unjudged.is_empty());
}

#[test]
fn a_build_with_another_compiled_package_format_still_gets_verdicts() {
    let r = Registry::new();
    r.add("coords", "1.0.0", "26.3", "");
    r.pin_source("coords", RELEASE_TAG);
    let root = r.0.path();
    let releases = root.join("releases.json");
    fs::write(
        &releases,
        json!([{"tag": RELEASE_TAG, "commit": RELEASE_PIN}]).to_string(),
    )
    .unwrap();
    let dir = root.join("sdk");
    sdk(
        &dir,
        RELEASE_PIN,
        mod_surface::SCHEMA,
        "api",
        API,
        json!({}),
    );
    let sdk_path = dir.join(RELEASE_PIN).join(sdk::INDEX);
    let mut sdk: Value = serde_json::from_slice(&fs::read(&sdk_path).unwrap()).unwrap();
    sdk["package_format"] = json!(sdk::PACKAGE_FORMAT + 1);
    fs::write(&sdk_path, sdk.to_string()).unwrap();

    let index = r
        .build(&[
            "--releases",
            releases.to_str().unwrap(),
            "--sdk-data",
            dir.to_str().unwrap(),
        ])
        .unwrap();
    let version = &index.project("coords").unwrap().versions[0];
    assert_eq!(
        serde_json::to_value(&version.verdicts).unwrap(),
        json!({(RELEASE_TAG): {"verdict": "made-for", "targets": []}})
    );
    assert!(index.unjudged.is_empty());
}

#[test]
fn an_unsupported_nightly_schema_leaves_stable_verdicts_and_lists_nightly_as_unjudged() {
    for unsupported_surface in [true, false] {
        let r = Registry::new();
        let root = r.0.path();
        r.add("coords", "1.0.0", "26.3", "");
        r.pin_source("coords", RELEASE_TAG);
        let releases = root.join("releases.json");
        fs::write(
            &releases,
            json!([{"tag": RELEASE_TAG, "commit": RELEASE_PIN}]).to_string(),
        )
        .unwrap();
        let nightly = "b".repeat(40);
        let nightly_file = root.join("nightly-build.json");
        fs::write(
            &nightly_file,
            json!({"date": DAY, "commit": nightly}).to_string(),
        )
        .unwrap();
        let dir = root.join("sdk");
        sdk(
            &dir,
            RELEASE_PIN,
            mod_surface::SCHEMA,
            "api",
            API,
            json!({}),
        );
        let surface_schema = if unsupported_surface {
            mod_surface::SCHEMA - 1
        } else {
            mod_surface::SCHEMA
        };
        sdk(&dir, &nightly, surface_schema, "api", API, json!({}));
        let (file, schema) = if unsupported_surface {
            ("surface.json", surface_schema)
        } else {
            let schema = build_data::SCHEMA + 1;
            fs::write(
                dir.join(&nightly)
                    .join("build-data")
                    .join(build_data::FUNCTIONS),
                json!({"schema": schema, "build": {"commit": nightly}}).to_string(),
            )
            .unwrap();
            (build_data::FUNCTIONS, schema)
        };
        let index = r
            .build(&[
                "--releases",
                releases.to_str().unwrap(),
                "--nightly-build",
                nightly_file.to_str().unwrap(),
                "--sdk-data",
                dir.to_str().unwrap(),
            ])
            .unwrap();
        let version = &index.project("coords").unwrap().versions[0];
        assert_eq!(
            serde_json::to_value(&version.verdicts).unwrap(),
            json!({(RELEASE_TAG): {"verdict": "made-for", "targets": []}})
        );
        let [unjudged] = index.unjudged.as_slice() else {
            panic!("expected one unjudged build, got {:?}", index.unjudged);
        };
        assert_eq!(unjudged.build, "nightly");
        assert!(unjudged.reason.contains(file), "{}", unjudged.reason);
        assert!(
            unjudged.reason.contains(&format!("schema {schema}")),
            "{}",
            unjudged.reason
        );
    }
}

#[test]
fn missing_or_inconsistent_sdk_files_still_fail_by_build_name() {
    let r = Registry::new();
    r.add("coords", "1.0.0", "26.3", "");
    r.pin_source("coords", RELEASE_TAG);
    let releases = r.0.path().join("releases.json");
    fs::write(
        &releases,
        json!([{"tag": RELEASE_TAG, "commit": RELEASE_PIN}]).to_string(),
    )
    .unwrap();
    let dir = r.0.path().join("sdk");
    sdk(
        &dir,
        RELEASE_PIN,
        mod_surface::SCHEMA - 1,
        "api",
        API,
        json!({}),
    );
    let flags = [
        "--releases",
        releases.to_str().unwrap(),
        "--sdk-data",
        dir.to_str().unwrap(),
    ];
    let assert_error = |file: &str| {
        let err = r.build(&flags).unwrap_err();
        for word in [RELEASE_TAG, RELEASE_PIN, file] {
            assert!(err.contains(word), "{err}");
        }
        err
    };

    let loops = dir
        .join(RELEASE_PIN)
        .join("build-data")
        .join(build_data::LOOPS);
    let loops_bytes = fs::read(&loops).unwrap();
    fs::remove_file(&loops).unwrap();
    assert_error(build_data::LOOPS);
    fs::write(loops, loops_bytes).unwrap();

    // A surface schema mismatch must not hide a wrong commit in a build-data document.
    let wrong = "b".repeat(40);
    fs::write(
        dir.join(RELEASE_PIN)
            .join("build-data")
            .join(build_data::INTERNALS),
        json!({"schema": build_data::SCHEMA + 1, "build": {"commit": wrong}}).to_string(),
    )
    .unwrap();
    assert!(assert_error(build_data::INTERNALS).contains(&wrong));

    let surface = dir.join(RELEASE_PIN).join("surface.json");
    // Keep the size and schema, but invalidate the hash in sdk.json.
    let text = fs::read_to_string(&surface).unwrap().replace("api", "bad");
    fs::write(&surface, text).unwrap();
    assert!(assert_error("surface.json").contains("checksum"));

    fs::remove_file(surface).unwrap();
    assert_error("surface.json");
}
