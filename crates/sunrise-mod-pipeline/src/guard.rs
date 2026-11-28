//! Runs mod-pipeline GLSL translation and validation on a 64 MiB stack thread ([`guarded`]).
//! The lexical GLSL nesting and operator-chain guard lives in [`sunrise_glsl::guard`].

/// Stack size of the checking thread; address space committed lazily by the OS.
const STACK_BYTES: usize = 64 << 20;

/// Runs `work` on a thread with a 64 MiB stack and returns its result, or why it failed (the
/// thread failed to spawn or `work` panicked).
pub fn guarded<T: Send>(work: impl FnOnce() -> T + Send) -> Result<T, String> {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .name("glsl-pipeline-check".to_owned())
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, work)
            .map_err(|e| format!("the shader check could not start: {e}"))?;
        thread.join().map_err(|panic| {
            format!(
                "the shader check panicked: {}",
                panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or(panic.downcast_ref::<&str>().copied())
                    .unwrap_or("no reason given")
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_glsl::guard::{MAX_EXPANDED_BYTES, refusal};

    #[test]
    fn guarded_runs_on_large_stack_and_catches_panics() {
        fn deep(n: usize) -> usize {
            let pad = [n as u8; 1024];
            if n == 0 {
                0
            } else {
                deep(n - 1) + usize::from(pad[0] & 1)
            }
        }
        assert_eq!(guarded(|| deep(8_192).min(1)), Ok(1));
        assert_eq!(
            guarded(|| -> () { panic!("boom") }),
            Err("the shader check panicked: boom".to_owned())
        );
    }

    /// Daily test (§6 & §13 Q1): every vanilla 26.3 core shader (expanded with its includes)
    /// passes `guard::refusal` with comfortable margin under `MAX_EXPANDED_BYTES`.
    #[test]
    #[ignore = "daily"]
    fn vanilla_core_shaders_pass_guard_with_margin() {
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .expect("SUNRISE_TEST_RESOURCES must point to prepared 26.3 resources");
        let core_dir = root.join("assets/minecraft/shaders/core");
        let read = |id: &str| -> Result<Option<String>, String> {
            let Some((ns, rest)) = id.split_once(':') else {
                return Ok(None);
            };
            let path = root.join("assets").join(ns).join(rest);
            match std::fs::read_to_string(&path) {
                Ok(text) => Ok(Some(text)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(format!("{}: {e}", path.display())),
            }
        };
        let includes = sunrise_glsl::Includes {
            read: &read,
            namespace: "minecraft",
            folder: "shaders/include/",
            once: &["moj_import"],
        };
        let mut count = 0usize;
        let mut largest_name = String::new();
        let mut largest_bytes = 0usize;
        for entry in std::fs::read_dir(&core_dir).expect("read vanilla core shaders dir") {
            let path = entry.unwrap().path();
            let ext = path.extension().and_then(|e| e.to_str());
            if !matches!(ext, Some("vsh" | "fsh")) {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let file_id = format!(
                "minecraft:shaders/core/{}",
                path.file_name().unwrap().to_string_lossy()
            );
            let expanded = sunrise_glsl::expand_includes(
                &text,
                &file_id,
                "minecraft:shaders/core/",
                &includes,
                Some(&[crate::DEPTH_ZERO_TO_ONE]),
            )
            .unwrap_or_else(|e| panic!("{}: expand failed: {e}", path.display()))
            .text()
            .to_owned();
            assert_eq!(
                refusal(&expanded),
                None,
                "vanilla core shader {} failed guard",
                path.display()
            );
            count += 1;
            if expanded.len() > largest_bytes {
                largest_bytes = expanded.len();
                largest_name = path.file_name().unwrap().to_string_lossy().into_owned();
            }
        }
        assert!(count > 0, "expected vanilla core shaders in {core_dir:?}");
        // Vanilla's largest core shader must sit well under MAX_EXPANDED_BYTES (at least 10x margin).
        assert!(
            largest_bytes * 10 < MAX_EXPANDED_BYTES,
            "largest vanilla core shader {largest_name} is {largest_bytes} bytes (budget {MAX_EXPANDED_BYTES})"
        );
    }
}
