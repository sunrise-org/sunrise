//! `frame-plan-measure` through its command line: a run measured, an empty run, an unreadable frame, no argument.

use std::path::{Path, PathBuf};
use std::process::Command;

use sunrise_frame_plan::model::{EventKind, Levels};
use sunrise_frame_plan::record::{Recorded, Start, TextureNote};
use sunrise_frame_plan::trace::{ClearValue, Load, Op, Texture, TextureId, Touch, TraceFrame, Use};

const TOOL: &str = env!("CARGO_BIN_EXE_frame-plan-measure");

/// A fresh folder of this test's own, removed when dropped.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("frame-plan-measure-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch folder");
        Folder(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn touch(texture: u32, how: Use) -> Touch {
    Touch {
        slot: 0,
        texture: TextureId(texture),
        levels: Levels::BASE,
        channels: 0b1111,
        how,
    }
}

/// Texture 0 a free buffer side, 1 the pinned screen: a clear of 0, a draw into 0 the next frame's clear kills,
/// and a draw onto the screen.
fn frame(seq: u64) -> Recorded {
    let texture = |id| Texture {
        id: TextureId(id),
        levels: 1,
        channels: 4,
        bytes: 1000 + u64::from(id),
    };
    let run = |work, kind, removable, touches| Op::Run {
        work,
        kind,
        effects: false,
        removable,
        touches,
    };
    Recorded {
        seq,
        size: [16, 16],
        start: Start::Steady,
        frame: TraceFrame {
            textures: vec![texture(0), texture(1)],
            ops: vec![
                run(
                    0,
                    EventKind::Clear,
                    false,
                    vec![touch(0, Use::Clear(ClearValue([0; 4])))],
                ),
                run(
                    1,
                    EventKind::Work,
                    true,
                    vec![touch(0, Use::Attach(Load::Keep))],
                ),
                run(
                    2,
                    EventKind::Work,
                    false,
                    vec![touch(1, Use::Attach(Load::Clear(ClearValue([0; 4]))))],
                ),
            ],
        },
        textures: vec![
            TextureNote {
                id: TextureId(0),
                pinned: false,
                alias: None,
                label: "colortex1 A".to_owned(),
            },
            TextureNote {
                id: TextureId(1),
                pinned: true,
                alias: None,
                label: "screen".to_owned(),
            },
        ],
        works: vec![(1, "composite".to_owned())],
        blocks: Vec::new(),
        guards: Vec::new(),
        errors: Vec::new(),
    }
}

fn write(folder: &Path, name: &str, text: &str) {
    std::fs::write(folder.join(name), text).expect("written");
}

fn measure(args: &[&Path]) -> (i32, String) {
    let out = Command::new(TOOL)
        .args(args)
        .output()
        .expect("the tool runs");
    (
        out.status.code().expect("an exit code"),
        String::from_utf8(out.stdout).expect("text"),
    )
}

#[test]
#[ignore = "daily"]
fn a_run_is_measured_and_its_removals_named() {
    let run = Folder::new("run");
    write(&run.0, "frame-00000001.trace", &frame(1).write());
    write(&run.0, "frame-00000002.trace", &frame(2).write());
    // Not a frame: left alone.
    write(&run.0, "notes.txt", "anything");
    let (code, out) = measure(&[&run.0]);
    assert_eq!(code, 0, "{out}");
    let compact: String = out.split_whitespace().collect();
    assert!(compact.contains("\"files\":2,\"refused\":[]"), "{out}");
    // Both frames are one state; the draw is removed and the buffer, cleared only, is dead.
    assert!(
        compact.contains("\"removed\":[{\"state\":0,\"work\":1,\"label\":\"composite\"}]"),
        "{out}"
    );
    assert!(
        compact.contains(
            "\"savings\":{\"events\":1,\"clears_folded\":0,\"clears_dropped\":1,\"bytes\":1000}"
        ),
        "{out}"
    );
}

#[test]
#[ignore = "daily"]
fn an_empty_run_or_an_unreadable_frame_fails() {
    let empty = Folder::new("empty");
    let (code, out) = measure(&[&empty.0]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("\"files\":0"), "{out}");
    let bad = Folder::new("bad");
    write(&bad.0, "frame-00000001.trace", &frame(1).write());
    write(
        &bad.0,
        "frame-00000002.trace",
        "sunrise-frame-trace 1\nnonsense\n",
    );
    let (code, out) = measure(&[&bad.0]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("frame-00000002.trace"), "{out}");
}

#[test]
#[ignore = "daily"]
fn no_folder_is_a_usage_error() {
    let out = Command::new(TOOL).output().expect("the tool runs");
    assert_eq!(out.status.code(), Some(2));
}
