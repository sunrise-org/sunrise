//! `frame-plan-measure DIR...`: each DIR holds one run's recorded frames (`*.trace`, as a runtime's frame trace
//! writes them). Prints, per run, a JSON object: each frame's checks, and what the plan would remove with the
//! callbacks as declared (`unresolved`) and narrowed to nothing (`narrowed`, not sound: a ceiling).
//!
//! Exits 1 when a run has no frame, a file can't be read or parsed, a frame's block check fails, or a state's edit
//! check fails; 2 on a usage error.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sunrise_frame_plan::measure::{Callbacks, Report, Sized, measure};
use sunrise_frame_plan::record::Recorded;

fn main() -> ExitCode {
    let dirs: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if dirs.is_empty() || dirs.iter().any(|d| d.as_os_str() == "--help") {
        eprintln!("usage: frame-plan-measure DIR...  (each DIR: one run's *.trace frames)");
        return ExitCode::from(2);
    }
    let mut failed = false;
    let mut out = Vec::new();
    for dir in &dirs {
        let (json, ok) = run(dir);
        failed |= !ok;
        out.push(json);
    }
    println!("[{}]", out.join(",\n"));
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// One run's JSON, and whether every check of it held.
fn run(dir: &Path) -> (String, bool) {
    let name = json_string(&dir.display().to_string());
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "trace"))
            .collect(),
        Err(e) => {
            let error = json_string(&format!("can't read the folder: {e}"));
            return (format!("{{\"run\":{name},\"error\":{error}}}"), false);
        }
    };
    files.sort();
    let mut frames = Vec::new();
    let mut refused = Vec::new();
    for file in &files {
        match std::fs::read_to_string(file)
            .map_err(|e| e.to_string())
            .and_then(|text| Recorded::parse(&text).map_err(|e| e.to_string()))
        {
            Ok(frame) => frames.push(frame),
            Err(e) => refused.push(format!("{}: {e}", file.display())),
        }
    }
    frames.sort_by_key(|f| f.seq);
    let refused_json: Vec<String> = refused.iter().map(|r| json_string(r)).collect();
    let head = format!(
        "\"run\":{name},\"files\":{},\"refused\":[{}]",
        files.len(),
        refused_json.join(",")
    );
    if frames.is_empty() {
        return (
            format!("{{{head},\"error\":\"no frame was recorded\"}}"),
            false,
        );
    }
    let mut ok = refused.is_empty();
    let mut parts = vec![head];
    for (key, callbacks) in [
        ("unresolved", Callbacks::Unresolved),
        ("narrowed", Callbacks::Narrowed),
    ] {
        match measure(&frames, callbacks) {
            Ok(report) => {
                ok &= holds(&report);
                parts.push(format!("\"{key}\":{}", report_json(&report)));
            }
            Err(e) => {
                ok = false;
                parts.push(format!(
                    "\"{key}\":{{\"error\":{}}}",
                    json_string(&e.to_string())
                ));
            }
        }
    }
    (format!("{{{}}}", parts.join(",")), ok)
}

fn holds(report: &Report) -> bool {
    report.sizes.iter().all(|sized| {
        sized
            .frames
            .iter()
            .all(|f| f.blocks.is_ok() && f.errors.is_empty())
            && sized.states.iter().all(|(_, edits)| edits.is_ok())
    })
}

fn report_json(report: &Report) -> String {
    let sizes: Vec<String> = report.sizes.iter().map(sized_json).collect();
    format!("{{\"sizes\":[{}]}}", sizes.join(","))
}

fn sized_json(report: &Sized) -> String {
    let frames: Vec<String> = report
        .frames
        .iter()
        .map(|f| {
            let errors: Vec<String> = f.errors.iter().map(|e| json_string(e)).collect();
            let guards: Vec<String> = f
                .guards
                .iter()
                .map(|(name, value)| format!("[{},{value}]", json_string(name)))
                .collect();
            format!(
                "{{\"seq\":{},\"state\":{},\"blocks\":{},\"errors\":[{}],\"guards\":[{}]}}",
                f.seq,
                f.state,
                result_json(&f.blocks),
                errors.join(","),
                guards.join(",")
            )
        })
        .collect();
    let states: Vec<String> = report
        .states
        .iter()
        .map(|(work, edits)| {
            format!(
                "{{\"passes\":{},\"clears\":{},\"blocks\":{},\"edits\":{}}}",
                work.passes,
                work.clears,
                work.blocks,
                result_json(edits)
            )
        })
        .collect();
    let removed: Vec<String> = report
        .removed
        .iter()
        .map(|r| {
            format!(
                "{{\"state\":{},\"work\":{},\"label\":{}}}",
                r.state,
                r.work,
                json_string(&r.label)
            )
        })
        .collect();
    let folded: Vec<String> = report
        .folded
        .iter()
        .map(|f| {
            format!(
                "{{\"state\":{},\"clear\":{},\"texture\":{},\"draw\":{},\"draw_label\":{}}}",
                f.state,
                f.clear,
                f.texture.0,
                f.draw,
                json_string(&f.draw_label)
            )
        })
        .collect();
    let dead: Vec<String> = report
        .dead
        .iter()
        .map(|d| {
            format!(
                "{{\"texture\":{},\"label\":{},\"bytes\":{}}}",
                d.texture.0,
                json_string(&d.label),
                d.bytes
            )
        })
        .collect();
    let s = report.savings;
    format!(
        "{{\"size\":[{},{}],\"frames\":[{}],\"states\":[{}],\"allocated\":{},\"plannable\":{},\"dead_bytes\":{},\
         \"removed\":[{}],\"folded\":[{}],\"dead\":[{}],\
         \"savings\":{{\"events\":{},\"clears_folded\":{},\"clears_dropped\":{},\"bytes\":{}}}}}",
        report.size[0],
        report.size[1],
        frames.join(","),
        states.join(","),
        report.allocated,
        report.plannable,
        report.dead_bytes,
        removed.join(","),
        folded.join(","),
        dead.join(","),
        s.events,
        s.clears_folded,
        s.clears_dropped,
        s.bytes
    )
}

fn result_json<E: std::fmt::Debug>(result: &Result<(), E>) -> String {
    match result {
        Ok(()) => "\"ok\"".to_owned(),
        Err(e) => json_string(&format!("{e:?}")),
    }
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
