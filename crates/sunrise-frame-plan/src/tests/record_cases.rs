//! The recorded frame's line format: a round trip of every field, and each refusal by kind and line.

use crate::model::{EventKind, Levels};
use crate::record::{BlockNote, ParseError, ParseErrorKind, Recorded, Start, TextureNote};
use crate::trace::{ClearValue, Load, Op, Span, Texture, TextureId, Touch, TraceFrame, Use};

fn touch(slot: u32, texture: u32, first: u8, count: u8, channels: u8, how: Use) -> Touch {
    Touch {
        slot,
        texture: TextureId(texture),
        levels: Levels { first, count },
        channels,
        how,
    }
}

/// A frame with every kind of line and field.
fn everything() -> Recorded {
    Recorded {
        seq: 41,
        size: [854, 480],
        start: Start::Resize,
        frame: TraceFrame {
            textures: vec![
                Texture {
                    id: TextureId(3),
                    levels: 1,
                    channels: 4,
                    bytes: 1_639_680,
                },
                Texture {
                    id: TextureId(9),
                    levels: 3,
                    channels: 2,
                    bytes: 77,
                },
            ],
            ops: vec![
                Op::Run {
                    work: 0,
                    kind: EventKind::Clear,
                    effects: false,
                    removable: false,
                    touches: vec![touch(
                        1,
                        3,
                        0,
                        1,
                        0b1111,
                        Use::Clear(ClearValue([1, 2, 3, 4])),
                    )],
                },
                Op::BlockStart(2),
                Op::Run {
                    work: 1,
                    kind: EventKind::Work,
                    effects: true,
                    removable: false,
                    touches: vec![
                        touch(0, 9, 1, 2, 0b11, Use::Sample),
                        touch(4, 9, 0, 1, 0b01, Use::Storage),
                        touch(0, 3, 0, 1, 0b1111, Use::Attach(Load::Keep)),
                    ],
                },
                Op::BlockEnd(2),
                Op::Run {
                    work: 5,
                    kind: EventKind::Copy,
                    effects: false,
                    removable: false,
                    touches: vec![
                        touch(0, 3, 0, 1, 0b1111, Use::CopyFrom),
                        touch(1, 9, 0, 1, 0b11, Use::CopyTo),
                    ],
                },
                Op::Run {
                    work: 6,
                    kind: EventKind::Work,
                    effects: false,
                    removable: true,
                    touches: vec![touch(
                        2,
                        3,
                        0,
                        1,
                        0b0111,
                        Use::Attach(Load::Clear(ClearValue([0, 0, 0, 0x3f80_0000]))),
                    )],
                },
                Op::Run {
                    work: 7,
                    kind: EventKind::Opaque,
                    effects: true,
                    removable: false,
                    touches: Vec::new(),
                },
            ],
        },
        textures: vec![
            TextureNote {
                id: TextureId(3),
                pinned: false,
                alias: Some(TextureId(9)),
                label: "colortex3 B".to_owned(),
            },
            TextureNote {
                id: TextureId(9),
                pinned: true,
                alias: None,
                label: "shadow depth".to_owned(),
            },
        ],
        works: vec![(0, "clear".to_owned()), (6, "composite 2".to_owned())],
        blocks: vec![BlockNote {
            block: 2,
            label: "opaque gbuffers".to_owned(),
            declared: vec![Span {
                texture: TextureId(9),
                levels: Levels { first: 0, count: 3 },
                channels: 0b11,
            }],
            unresolved: vec![TextureId(3)],
        }],
        guards: vec![
            ("shadow cache hit".to_owned(), true),
            ("callback drew".to_owned(), false),
        ],
        errors: vec!["a write into texture view 12 the frame doesn't list".to_owned()],
    }
}

#[test]
fn a_frame_reads_back_as_it_was_written() {
    let frame = everything();
    assert_eq!(Recorded::parse(&frame.write()), Ok(frame));
}

#[test]
fn a_label_with_a_line_break_stays_on_its_line() {
    let mut frame = everything();
    frame.works[0].1 = "two\nlines".to_owned();
    let read = Recorded::parse(&frame.write()).expect("parses");
    assert_eq!(read.works[0].1, "two lines");
}

/// `text` with line `line` (from 1) replaced by `with`, or removed for `None`.
fn edited(line: usize, with: Option<&str>) -> String {
    let text = everything().write();
    let mut lines: Vec<&str> = text.lines().collect();
    match with {
        Some(w) => lines[line - 1] = w,
        None => _ = lines.remove(line - 1),
    }
    lines.join("\n")
}

fn refusal(text: &str) -> ParseError {
    Recorded::parse(text).expect_err("refused")
}

fn at(line: usize, kind: ParseErrorKind) -> ParseError {
    ParseError { line, kind }
}

#[test]
fn each_malformed_line_is_refused_by_kind_and_line() {
    // Line 1 is the header, 2 the frame, 3 and 4 the textures, 5 and 6 the works, 7 the block, 8 its span, 9 its
    // unresolved texture, 10 the first run, 11 its touch.
    assert_eq!(
        refusal(&edited(1, Some("sunrise-frame-trace 2"))),
        at(1, ParseErrorKind::Header)
    );
    assert_eq!(refusal(""), at(1, ParseErrorKind::Header));
    let lines = everything().write().lines().count();
    assert_eq!(
        refusal(&edited(2, None)),
        at(lines - 1, ParseErrorKind::Frame)
    );
    assert_eq!(
        refusal(&edited(3, Some("frame 1 2 3 steady"))),
        at(3, ParseErrorKind::Frame)
    );
    assert_eq!(
        refusal(&edited(3, Some("buffer 1"))),
        at(3, ParseErrorKind::UnknownLine)
    );
    assert_eq!(
        refusal(&edited(2, Some("frame 41 854 480"))),
        at(2, ParseErrorKind::Fields)
    );
    assert_eq!(
        refusal(&edited(2, Some("frame 41 854 480 resize extra"))),
        at(2, ParseErrorKind::Fields)
    );
    assert_eq!(
        refusal(&edited(2, Some("frame 41 854 480 sideways"))),
        at(2, ParseErrorKind::Field(3))
    );
    assert_eq!(
        refusal(&edited(
            3,
            Some("texture 3 one 4 1639680 free 9 colortex3 B")
        )),
        at(3, ParseErrorKind::Field(1))
    );
    assert_eq!(
        refusal(&edited(
            3,
            Some("texture 3 1 4 1639680 maybe 9 colortex3 B")
        )),
        at(3, ParseErrorKind::Field(4))
    );
    assert_eq!(
        refusal(&edited(4, Some("texture 3 3 2 77 pinned - again"))),
        at(4, ParseErrorKind::Duplicate)
    );
    assert_eq!(
        refusal(&edited(6, Some("work 0 again"))),
        at(6, ParseErrorKind::Duplicate)
    );
    assert_eq!(
        refusal(&edited(8, Some("declare 5 9 0 3 3"))),
        at(8, ParseErrorKind::Unknown)
    );
    assert_eq!(
        refusal(&edited(10, Some("run 0 draw 0 0"))),
        at(10, ParseErrorKind::Field(1))
    );
    assert_eq!(
        refusal(&edited(10, Some("run 0 clear 2 0"))),
        at(10, ParseErrorKind::Field(2))
    );
    assert_eq!(
        refusal(&edited(10, Some("start 3"))),
        at(11, ParseErrorKind::TouchOutsideRun)
    );
    assert_eq!(
        refusal(&edited(11, Some("touch 1 3 0 1 15 clear 1 2 3"))),
        at(11, ParseErrorKind::Fields)
    );
    assert_eq!(
        refusal(&edited(11, Some("touch 1 3 0 1 15 sample 1"))),
        at(11, ParseErrorKind::Fields)
    );
    assert_eq!(
        refusal(&edited(11, Some("touch 1 3 0 1 15 smudge"))),
        at(11, ParseErrorKind::Field(5))
    );
    assert_eq!(
        refusal(&edited(11, Some("guard 2 a guard"))),
        at(11, ParseErrorKind::Field(0))
    );
    assert_eq!(
        refusal(&edited(11, Some("guard 1"))),
        at(11, ParseErrorKind::Fields)
    );
}

#[test]
fn a_note_naming_a_texture_the_frame_does_not_list_is_refused() {
    let mut alias = everything();
    alias.textures[0].alias = Some(TextureId(40));
    let lines = alias.write().lines().count();
    assert_eq!(refusal(&alias.write()), at(lines, ParseErrorKind::Unknown));
    let mut declared = everything();
    declared.blocks[0].unresolved.push(TextureId(41));
    let lines = declared.write().lines().count();
    assert_eq!(
        refusal(&declared.write()),
        at(lines, ParseErrorKind::Unknown)
    );
}
