//! Runtime native outputs come from Cargo messages, never guessed linker arguments.
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, Write},
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
enum CargoEvent {
    BuildScriptExecuted {
        package_id: String,
        linked_paths: Vec<String>,
    },
    CompilerArtifact {
        package_id: String,
        filenames: Vec<PathBuf>,
        target: Target,
    },
    CompilerMessage {
        message: crate::present::Diagnostic,
    },
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
struct Target {
    kind: Vec<String>,
}
#[derive(Default)]
pub struct Outputs {
    scripts: Vec<(String, Vec<String>)>,
    libraries: Vec<(String, Vec<PathBuf>)>,
    pub errors: Vec<crate::present::Diagnostic>,
}

fn dynamic(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    [".so", ".dll", ".dylib"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
        || name.contains(".so.")
}

impl Outputs {
    fn push_line(&mut self, line: &str) -> Result<Option<&str>, String> {
        if line.is_empty() {
            return Ok(None);
        }
        match serde_json::from_str(line).map_err(|e| format!("Cargo build messages: {e}"))? {
            CargoEvent::BuildScriptExecuted {
                package_id,
                linked_paths,
            } => {
                self.scripts.push((package_id, linked_paths));
                Ok(None)
            }
            CargoEvent::CompilerArtifact {
                package_id,
                filenames,
                target,
            } if !target.kind.iter().any(|kind| kind == "proc-macro") => {
                self.libraries.push((package_id, filenames));
                Ok(None)
            }
            CargoEvent::CompilerMessage { message } if message.rendered().is_some() => {
                self.errors.push(message);
                Ok(self.errors.last().and_then(|message| message.rendered()))
            }
            _ => Ok(None),
        }
    }

    /// Reads Cargo's JSON message stream once: copies every raw line to `raw_out`, collects
    /// runtime outputs and diagnostics, and calls `on_rendered` for each compiler message as it
    /// arrives. Drains to EOF even after a parse error so Cargo never blocks on a closed pipe.
    pub(crate) fn stream(
        mut reader: impl BufRead,
        raw_out: &mut impl Write,
        mut on_rendered: impl FnMut(&str),
    ) -> Result<Self, String> {
        let mut outputs = Self::default();
        let mut bytes = Vec::new();
        let mut first_error: Option<String> = None;
        loop {
            bytes.clear();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) => {
                    first_error.get_or_insert_with(|| error.to_string());
                    break;
                }
            }
            if let Err(error) = raw_out.write_all(&bytes) {
                first_error.get_or_insert_with(|| error.to_string());
            }
            if first_error.is_none() {
                let line = if let Some(line) = bytes.strip_suffix(b"\n") {
                    line.strip_suffix(b"\r").unwrap_or(line)
                } else {
                    &bytes
                };
                match std::str::from_utf8(line)
                    .map_err(|e| format!("Cargo build messages: {e}"))
                    .and_then(|text| outputs.push_line(text))
                {
                    Ok(Some(rendered)) => on_rendered(rendered),
                    Ok(None) => {}
                    Err(error) => first_error = Some(error),
                }
            }
        }
        if let Err(error) = raw_out.flush() {
            first_error.get_or_insert_with(|| error.to_string());
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(outputs),
        }
    }

    #[cfg(test)]
    pub fn read(path: &Path) -> Result<Self, String> {
        let mut outputs = Self::default();
        for line in
            std::io::BufReader::new(fs::File::open(path).map_err(|e| e.to_string())?).lines()
        {
            outputs.push_line(&line.map_err(|e| e.to_string())?)?;
        }
        Ok(outputs)
    }

    pub fn runtime(
        &self,
        runtime: &BTreeSet<String>,
        target: &Path,
        cargo_home: &Path,
    ) -> Result<Vec<PathBuf>, String> {
        let roots = [target.to_path_buf(), cargo_home.to_path_buf()]
            .into_iter()
            .filter(|root| root.exists())
            .map(|root| root.canonicalize().map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut candidates = Vec::new();
        for (id, files) in &self.libraries {
            if runtime.contains(id) {
                candidates.extend(files.iter().filter(|file| dynamic(file)).cloned());
            }
        }
        for (_, paths) in self.scripts.iter().filter(|(id, _)| runtime.contains(id)) {
            for search in paths {
                let (kind, path) = search.split_once('=').unwrap_or(("native", search));
                let path = Path::new(path);
                if !path.is_absolute() {
                    return Err(format!(
                        "Native dependency emitted relative library search path {}",
                        path.display()
                    ));
                }
                let canonical = path
                    .canonicalize()
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                if !roots.iter().any(|root| canonical.starts_with(root)) {
                    continue;
                } // System libraries stay system libraries.
                if kind == "framework" {
                    return Err(format!(
                        "Bundled framework output {} is not supported by native mod packaging",
                        path.display()
                    ));
                }
                for entry in fs::read_dir(canonical).map_err(|e| e.to_string())? {
                    let file = entry.map_err(|e| e.to_string())?.path();
                    if dynamic(&file) {
                        candidates.push(file);
                    }
                }
            }
        }
        let mut files = BTreeMap::new();
        for path in candidates {
            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
            if !roots.iter().any(|root| canonical.starts_with(root)) {
                return Err(format!(
                    "Native output {} escapes the managed build",
                    path.display()
                ));
            }
            let name = path
                .file_name()
                .ok_or("Native output has no filename")?
                .to_owned();
            if let Some(previous) = files.get(&name) {
                if fs::read(previous).map_err(|e| e.to_string())?
                    != fs::read(&path).map_err(|e| e.to_string())?
                {
                    return Err(format!(
                        "Conflicting native runtime outputs named {}",
                        name.to_string_lossy()
                    ));
                }
            } else {
                files.insert(name, path);
            }
        }
        Ok(files.into_values().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_outputs_exclude_host_tools_and_preserve_native_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        // read_dir returns canonical-prefixed paths on Windows. Match that representation
        // without canonicalizing individual files, which would erase Unix aliases.
        fs::create_dir(&target).unwrap();
        let target = target.canonicalize().unwrap();
        let library = target.join("libnative.so.1");
        fs::write(&library, b"native").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&library, target.join("libnative.so")).unwrap();
        let host = dir.path().join("host.so");
        fs::write(&host, b"host").unwrap();
        let messages = dir.path().join("messages");
        let rows = [
            serde_json::json!({"reason":"build-script-executed","package_id":"runtime","linked_paths":[format!("native={}",target.display())]}),
            serde_json::json!({"reason":"compiler-artifact","package_id":"host","filenames":[host],"target":{"kind":["proc-macro"]}}),
        ];
        fs::write(
            &messages,
            rows.iter()
                .map(|row| row.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        let files = Outputs::read(&messages)
            .unwrap()
            .runtime(
                &BTreeSet::from(["runtime".into()]),
                &target,
                &target.join("cargo-home"),
            )
            .unwrap();
        assert!(files.contains(&library));
        assert!(!files.contains(&host));
        #[cfg(unix)]
        assert!(files.contains(&target.join("libnative.so")));
    }
    #[test]
    fn escaping_and_conflicting_runtime_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir_all(target.join("one")).unwrap();
        fs::create_dir_all(target.join("two")).unwrap();
        let one = target.join("one/library.so");
        let two = target.join("two/library.so");
        fs::write(&one, b"one").unwrap();
        fs::write(&two, b"two").unwrap();
        let mut outputs = Outputs {
            scripts: vec![],
            libraries: vec![("runtime".into(), vec![one, two])],
            errors: vec![],
        };
        let runtime = BTreeSet::from(["runtime".into()]);
        assert!(
            outputs
                .runtime(&runtime, &target, &target.join("cargo-home"))
                .unwrap_err()
                .contains("Conflicting")
        );
        let outside = dir.path().join("outside.so");
        fs::write(&outside, b"outside").unwrap();
        outputs.libraries[0].1 = vec![outside];
        assert!(
            outputs
                .runtime(&runtime, &target, &target.join("cargo-home"))
                .unwrap_err()
                .contains("escapes")
        );
        outputs.libraries.clear();
        outputs.scripts = vec![("runtime".into(), vec!["native=relative".into()])];
        assert!(
            outputs
                .runtime(&runtime, &target, &target.join("cargo-home"))
                .unwrap_err()
                .contains("relative")
        );
    }

    #[test]
    fn stream_mirrors_compiler_messages_before_eof_and_preserves_raw_jsonl() {
        use std::io::Read;
        use std::sync::mpsc;

        struct GatedReader {
            first: Vec<u8>,
            second: Vec<u8>,
            stage: u8,
            ack: mpsc::Receiver<()>,
        }

        impl Read for GatedReader {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let source = match self.stage {
                    0 => {
                        self.stage = 1;
                        &self.first
                    }
                    1 => {
                        self.ack
                            .try_recv()
                            .expect("compiler-message must be mirrored before stream EOF");
                        self.stage = 2;
                        &self.second
                    }
                    _ => return Ok(0),
                };
                let n = source.len().min(buf.len());
                buf[..n].copy_from_slice(&source[..n]);
                Ok(n)
            }
        }

        let first = format!(
            "{}\n",
            serde_json::json!({
                "reason": "compiler-message",
                "message": {
                    "rendered": "warning: function `registry_probe` is never used\n",
                    "spans": []
                }
            })
        );
        let second = format!(
            "{}\n",
            serde_json::json!({
                "reason": "compiler-artifact",
                "package_id": "greeter",
                "filenames": ["/tmp/libgreeter.so"],
                "target": { "kind": ["cdylib"] }
            })
        );
        let (tx, rx) = mpsc::channel();
        let reader = std::io::BufReader::new(GatedReader {
            first: first.as_bytes().to_vec(),
            second: second.as_bytes().to_vec(),
            stage: 0,
            ack: rx,
        });
        let mut raw = Vec::new();
        let mut mirrored = Vec::new();
        let outputs = Outputs::stream(reader, &mut raw, |rendered| {
            mirrored.push(rendered.to_owned());
            tx.send(()).unwrap();
        })
        .unwrap();

        assert_eq!(
            mirrored,
            vec!["warning: function `registry_probe` is never used\n"]
        );
        assert_eq!(String::from_utf8(raw).unwrap(), format!("{first}{second}"));
        assert_eq!(outputs.errors.len(), 1);
        assert_eq!(outputs.libraries.len(), 1);
    }
}
