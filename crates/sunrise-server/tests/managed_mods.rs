use std::{fs, process::Command};

#[test]
#[ignore = "daily"]
fn start_and_run_refuse_drift_without_writing_the_server_folder() {
    const BUILD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let settings = "[mods]\nlist = ['rails']\n";
    fs::write(dir.join("server.toml"), settings).unwrap();
    let artifact = b"a different checksum";
    let lock = format!(
        r#"
format = 1
sunrise = '{BUILD}'
input = {{ list = ['rails'] }}
[[mods]]
id = 'rails'
version = '1.0.0'
url = 'https://example.net/rails.smod'
sha256 = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
file = 'rails.smod'
reason = 'requested'
on = true
"#
    );
    for (state, files) in [("missing lock", 1), ("missing mod", 2), ("changed mod", 3)] {
        if files == 2 {
            fs::write(dir.join("mods.lock"), &lock).unwrap();
        } else if files == 3 {
            fs::write(dir.join("rails.smod"), artifact).unwrap();
        }
        for verb in ["start", "run"] {
            let output = Command::new(env!("CARGO_BIN_EXE_sunrise-server"))
                .arg("--dir")
                .arg(dir)
                .arg(verb)
                .output()
                .unwrap();
            let error = String::from_utf8(output.stderr).unwrap();
            assert_eq!(
                output.status.code(),
                Some(2),
                "{verb} with {state}: {error}"
            );
            if files >= 2 {
                assert_eq!(fs::read_to_string(dir.join("mods.lock")).unwrap(), lock);
                assert!(
                    error
                        .lines()
                        .last()
                        .unwrap()
                        .ends_with("sunrise-server mods update rails")
                );
            }
            if files == 3 {
                assert_eq!(fs::read(dir.join("rails.smod")).unwrap(), artifact);
            }
            assert_eq!(
                fs::read_to_string(dir.join("server.toml")).unwrap(),
                settings
            );
            assert_eq!(fs::read_dir(dir).unwrap().count(), files);
        }
    }
}
