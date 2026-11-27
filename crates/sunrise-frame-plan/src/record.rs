//! A frame as a runtime recorded it, with what the trace alone doesn't say: which textures the planner must leave
//! alone and which one a texture's bindings fall back to, what each unit of work and block is called, the access set
//! declared for each block, and how the frame followed the one before. A line format carries it from the runtime
//! to the measure ([`Recorded::write`], [`Recorded::parse`]).
//!
//! ```text
//! sunrise-frame-trace 1
//! frame SEQ WIDTH HEIGHT first|steady|reset|resize
//! texture ID LEVELS CHANNELS BYTES pinned|free ALIAS|- LABEL
//! work ID LABEL
//! block ID LABEL
//! declare BLOCK TEXTURE FIRST COUNT CHANNELS
//! unresolved BLOCK TEXTURE
//! run WORK work|clear|copy|opaque EFFECTS REMOVABLE
//! touch SLOT TEXTURE FIRST COUNT CHANNELS sample|storage|keep|copy-from|copy-to
//! touch SLOT TEXTURE FIRST COUNT CHANNELS load-clear|clear W0 W1 W2 W3
//! start BLOCK
//! end BLOCK
//! guard 0|1 NAME
//! error TEXT
//! ```
//!
//! `touch` lines belong to the `run` line before them. Labels and error texts run to the end of the line.

use crate::model::{EventKind, Levels};
use crate::trace::{ClearValue, Load, Op, Span, Texture, TextureId, Touch, TraceFrame, Use};

/// The first line of every recorded frame, with the format's version.
pub const HEADER: &str = "sunrise-frame-trace 1";

/// How a frame followed the one recorded before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// The first frame after the targets were made: no frame before it is kept.
    First,
    Steady,
    /// The runtime forgot what earlier frames kept.
    Reset,
    /// The render size changed; what earlier frames kept stays where it can.
    Resize,
}

/// What the planner needs to know of a texture beyond its size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureNote {
    pub id: TextureId,
    /// Never planned on (the runtime's own exclusions).
    pub pinned: bool,
    /// The texture its bindings use once it is dead.
    pub alias: Option<TextureId>,
    pub label: String,
}

/// A block: its label, the access set declared for it, and the textures it may reach that its runtime can't
/// resolve (a callback into other code).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockNote {
    pub block: u32,
    pub label: String,
    pub declared: Vec<Span>,
    pub unresolved: Vec<TextureId>,
}

/// One recorded frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    /// The frame's number in the run: consecutive frames have consecutive numbers.
    pub seq: u64,
    pub size: [u32; 2],
    pub start: Start,
    pub frame: TraceFrame,
    /// One per texture of `frame.textures`.
    pub textures: Vec<TextureNote>,
    /// Each unit of work's label.
    pub works: Vec<(u32, String)>,
    /// One per block the frame ran.
    pub blocks: Vec<BlockNote>,
    /// The value each named guard had in this frame (a callback's result, a cache hit, a size-skipped pass).
    pub guards: Vec<(String, bool)>,
    /// What the runtime could not record (a write into a texture it doesn't know).
    pub errors: Vec<String>,
}

/// A recorded frame the parser refuses: the line (from 1) and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub kind: ParseErrorKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// The first line isn't [`HEADER`].
    Header,
    /// No `frame` line, or a second one.
    Frame,
    /// A line of an unknown kind.
    UnknownLine,
    /// A line with too few or too many fields.
    Fields,
    /// A field that isn't a number, or a word that isn't one of its line's.
    Field(usize),
    /// A `touch` before any `run`.
    TouchOutsideRun,
    /// A texture, work or block named twice.
    Duplicate,
    /// A note naming a texture or block the frame doesn't list.
    Unknown,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "recorded frame refused at line {}: {:?}",
            self.line, self.kind
        )
    }
}

impl std::error::Error for ParseError {}

fn kind_word(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Work => "work",
        EventKind::Clear => "clear",
        EventKind::Copy => "copy",
        EventKind::Opaque => "opaque",
    }
}

fn start_word(start: Start) -> &'static str {
    match start {
        Start::First => "first",
        Start::Steady => "steady",
        Start::Reset => "reset",
        Start::Resize => "resize",
    }
}

/// A label on one line: line breaks become spaces, and an empty one is `-`.
fn one_line(label: &str) -> String {
    let line: String = label
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    if line.trim().is_empty() {
        "-".to_owned()
    } else {
        line.trim().to_owned()
    }
}

impl Recorded {
    /// The frame in the line format.
    pub fn write(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let [w, h] = self.size;
        let _ = writeln!(out, "{HEADER}");
        let _ = writeln!(out, "frame {} {w} {h} {}", self.seq, start_word(self.start));
        for texture in &self.frame.textures {
            let note = self.textures.iter().find(|n| n.id == texture.id);
            let (pinned, alias, label) = match note {
                Some(n) => (n.pinned, n.alias, n.label.as_str()),
                None => (true, None, "-"),
            };
            let alias = alias.map_or("-".to_owned(), |a| a.0.to_string());
            let _ = writeln!(
                out,
                "texture {} {} {} {} {} {alias} {}",
                texture.id.0,
                texture.levels,
                texture.channels,
                texture.bytes,
                if pinned { "pinned" } else { "free" },
                one_line(label)
            );
        }
        for (work, label) in &self.works {
            let _ = writeln!(out, "work {work} {}", one_line(label));
        }
        for block in &self.blocks {
            let _ = writeln!(out, "block {} {}", block.block, one_line(&block.label));
            for span in &block.declared {
                let _ = writeln!(
                    out,
                    "declare {} {} {} {} {}",
                    block.block,
                    span.texture.0,
                    span.levels.first,
                    span.levels.count,
                    span.channels
                );
            }
            for texture in &block.unresolved {
                let _ = writeln!(out, "unresolved {} {}", block.block, texture.0);
            }
        }
        for op in &self.frame.ops {
            match op {
                Op::Run {
                    work,
                    kind,
                    effects,
                    removable,
                    touches,
                } => {
                    let _ = writeln!(
                        out,
                        "run {work} {} {} {}",
                        kind_word(*kind),
                        u8::from(*effects),
                        u8::from(*removable)
                    );
                    for t in touches {
                        let how = match t.how {
                            Use::Sample => "sample".to_owned(),
                            Use::Storage => "storage".to_owned(),
                            Use::Attach(Load::Keep) => "keep".to_owned(),
                            Use::CopyFrom => "copy-from".to_owned(),
                            Use::CopyTo => "copy-to".to_owned(),
                            Use::Attach(Load::Clear(ClearValue(v))) => {
                                format!("load-clear {} {} {} {}", v[0], v[1], v[2], v[3])
                            }
                            Use::Clear(ClearValue(v)) => {
                                format!("clear {} {} {} {}", v[0], v[1], v[2], v[3])
                            }
                        };
                        let _ = writeln!(
                            out,
                            "touch {} {} {} {} {} {how}",
                            t.slot, t.texture.0, t.levels.first, t.levels.count, t.channels
                        );
                    }
                }
                Op::BlockStart(b) => {
                    let _ = writeln!(out, "start {b}");
                }
                Op::BlockEnd(b) => {
                    let _ = writeln!(out, "end {b}");
                }
            }
        }
        for (name, value) in &self.guards {
            let _ = writeln!(out, "guard {} {}", u8::from(*value), one_line(name));
        }
        for error in &self.errors {
            let _ = writeln!(out, "error {}", one_line(error));
        }
        out
    }

    /// Reads one frame in the line format.
    pub fn parse(text: &str) -> Result<Recorded, ParseError> {
        let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l));
        match lines.next() {
            Some((_, line)) if line.trim_end() == HEADER => {}
            _ => {
                return Err(ParseError {
                    line: 1,
                    kind: ParseErrorKind::Header,
                });
            }
        }
        let mut frame_line: Option<(u64, [u32; 2], Start)> = None;
        let mut recorded = Recorded {
            seq: 0,
            size: [0, 0],
            start: Start::Steady,
            frame: TraceFrame::default(),
            textures: Vec::new(),
            works: Vec::new(),
            blocks: Vec::new(),
            guards: Vec::new(),
            errors: Vec::new(),
        };
        for (at, line) in lines {
            let fail = |kind| ParseError { line: at, kind };
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
            // The fields before a free text, and the text.
            let fields = |count: usize| -> Result<(Vec<&str>, &str), ParseError> {
                let mut parts = rest.splitn(count + 1, ' ');
                let mut out = Vec::with_capacity(count);
                for _ in 0..count {
                    match parts.next() {
                        Some(p) if !p.is_empty() => out.push(p),
                        _ => return Err(fail(ParseErrorKind::Fields)),
                    }
                }
                Ok((out, parts.next().unwrap_or("")))
            };
            let exact = |count: usize| -> Result<Vec<&str>, ParseError> {
                let (out, tail) = fields(count)?;
                if tail.is_empty() {
                    Ok(out)
                } else {
                    Err(fail(ParseErrorKind::Fields))
                }
            };
            fn field<T: std::str::FromStr>(
                field: &str,
                index: usize,
                line: usize,
            ) -> Result<T, ParseError> {
                field.parse().map_err(|_| ParseError {
                    line,
                    kind: ParseErrorKind::Field(index),
                })
            }
            let flag = |field: &str, index: usize| match field {
                "0" => Ok(false),
                "1" => Ok(true),
                _ => Err(fail(ParseErrorKind::Field(index))),
            };
            match kind {
                "frame" => {
                    let f = exact(4)?;
                    if frame_line.is_some() {
                        return Err(fail(ParseErrorKind::Frame));
                    }
                    let start = match f[3] {
                        "first" => Start::First,
                        "steady" => Start::Steady,
                        "reset" => Start::Reset,
                        "resize" => Start::Resize,
                        _ => return Err(fail(ParseErrorKind::Field(3))),
                    };
                    frame_line = Some((
                        field(f[0], 0, at)?,
                        [field(f[1], 1, at)?, field(f[2], 2, at)?],
                        start,
                    ));
                }
                "texture" => {
                    let (f, label) = fields(6)?;
                    let id = TextureId(field(f[0], 0, at)?);
                    if recorded.frame.textures.iter().any(|t| t.id == id) {
                        return Err(fail(ParseErrorKind::Duplicate));
                    }
                    let pinned = match f[4] {
                        "pinned" => true,
                        "free" => false,
                        _ => return Err(fail(ParseErrorKind::Field(4))),
                    };
                    let alias = match f[5] {
                        "-" => None,
                        a => Some(TextureId(field(a, 5, at)?)),
                    };
                    recorded.frame.textures.push(Texture {
                        id,
                        levels: field(f[1], 1, at)?,
                        channels: field(f[2], 2, at)?,
                        bytes: field(f[3], 3, at)?,
                    });
                    recorded.textures.push(TextureNote {
                        id,
                        pinned,
                        alias,
                        label: label.to_owned(),
                    });
                }
                "work" => {
                    let (f, label) = fields(1)?;
                    let work: u32 = field(f[0], 0, at)?;
                    if recorded.works.iter().any(|(w, _)| *w == work) {
                        return Err(fail(ParseErrorKind::Duplicate));
                    }
                    recorded.works.push((work, label.to_owned()));
                }
                "block" => {
                    let (f, label) = fields(1)?;
                    let block: u32 = field(f[0], 0, at)?;
                    if recorded.blocks.iter().any(|b| b.block == block) {
                        return Err(fail(ParseErrorKind::Duplicate));
                    }
                    recorded.blocks.push(BlockNote {
                        block,
                        label: label.to_owned(),
                        declared: Vec::new(),
                        unresolved: Vec::new(),
                    });
                }
                "declare" => {
                    let f = exact(5)?;
                    let block: u32 = field(f[0], 0, at)?;
                    let span = Span {
                        texture: TextureId(field(f[1], 1, at)?),
                        levels: Levels {
                            first: field(f[2], 2, at)?,
                            count: field(f[3], 3, at)?,
                        },
                        channels: field(f[4], 4, at)?,
                    };
                    recorded
                        .blocks
                        .iter_mut()
                        .find(|b| b.block == block)
                        .ok_or(fail(ParseErrorKind::Unknown))?
                        .declared
                        .push(span);
                }
                "unresolved" => {
                    let f = exact(2)?;
                    let block: u32 = field(f[0], 0, at)?;
                    let texture = TextureId(field(f[1], 1, at)?);
                    recorded
                        .blocks
                        .iter_mut()
                        .find(|b| b.block == block)
                        .ok_or(fail(ParseErrorKind::Unknown))?
                        .unresolved
                        .push(texture);
                }
                "run" => {
                    let f = exact(4)?;
                    let kind = match f[1] {
                        "work" => EventKind::Work,
                        "clear" => EventKind::Clear,
                        "copy" => EventKind::Copy,
                        "opaque" => EventKind::Opaque,
                        _ => return Err(fail(ParseErrorKind::Field(1))),
                    };
                    recorded.frame.ops.push(Op::Run {
                        work: field(f[0], 0, at)?,
                        kind,
                        effects: flag(f[2], 2)?,
                        removable: flag(f[3], 3)?,
                        touches: Vec::new(),
                    });
                }
                "touch" => {
                    let (f, tail) = fields(6)?;
                    let value = |tail: &str| -> Result<ClearValue, ParseError> {
                        let words: Vec<&str> = tail.split(' ').collect();
                        if words.len() != 4 {
                            return Err(fail(ParseErrorKind::Fields));
                        }
                        let mut v = [0u32; 4];
                        for (i, w) in words.iter().enumerate() {
                            v[i] = field(w, 6 + i, at)?;
                        }
                        Ok(ClearValue(v))
                    };
                    let how = match f[5] {
                        "sample" | "storage" | "keep" | "copy-from" | "copy-to"
                            if !tail.is_empty() =>
                        {
                            return Err(fail(ParseErrorKind::Fields));
                        }
                        "sample" => Use::Sample,
                        "storage" => Use::Storage,
                        "keep" => Use::Attach(Load::Keep),
                        "copy-from" => Use::CopyFrom,
                        "copy-to" => Use::CopyTo,
                        "load-clear" => Use::Attach(Load::Clear(value(tail)?)),
                        "clear" => Use::Clear(value(tail)?),
                        _ => return Err(fail(ParseErrorKind::Field(5))),
                    };
                    let touch = Touch {
                        slot: field(f[0], 0, at)?,
                        texture: TextureId(field(f[1], 1, at)?),
                        levels: Levels {
                            first: field(f[2], 2, at)?,
                            count: field(f[3], 3, at)?,
                        },
                        channels: field(f[4], 4, at)?,
                        how,
                    };
                    match recorded.frame.ops.last_mut() {
                        Some(Op::Run { touches, .. }) => touches.push(touch),
                        _ => return Err(fail(ParseErrorKind::TouchOutsideRun)),
                    }
                }
                "start" => {
                    let f = exact(1)?;
                    recorded.frame.ops.push(Op::BlockStart(field(f[0], 0, at)?));
                }
                "end" => {
                    let f = exact(1)?;
                    recorded.frame.ops.push(Op::BlockEnd(field(f[0], 0, at)?));
                }
                "guard" => {
                    let (f, name) = fields(1)?;
                    if name.is_empty() {
                        return Err(fail(ParseErrorKind::Fields));
                    }
                    let value = flag(f[0], 0)?;
                    recorded.guards.push((name.to_owned(), value));
                }
                "error" => recorded.errors.push(rest.to_owned()),
                _ => return Err(fail(ParseErrorKind::UnknownLine)),
            }
        }
        let Some((seq, size, start)) = frame_line else {
            return Err(ParseError {
                line: text.lines().count().max(1),
                kind: ParseErrorKind::Frame,
            });
        };
        let known = |id: TextureId| recorded.frame.textures.iter().any(|t| t.id == id);
        let unknown_alias = recorded
            .textures
            .iter()
            .any(|n| n.alias.is_some_and(|a| !known(a)));
        let unknown_block_texture = recorded.blocks.iter().any(|b| {
            b.declared.iter().any(|s| !known(s.texture)) || b.unresolved.iter().any(|&t| !known(t))
        });
        if unknown_alias || unknown_block_texture {
            return Err(ParseError {
                line: text.lines().count().max(1),
                kind: ParseErrorKind::Unknown,
            });
        }
        recorded.seq = seq;
        recorded.size = size;
        recorded.start = start;
        Ok(recorded)
    }
}
