//! The checks a shader of the game's passes before the renderer builds it: naga
//! validation located in the file a refusal comes from, and translation for every backend
//! wgpu uses.
use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};
use std::fmt;

/// What is wrong, and where: a file, with the line and column when known (0 when not).
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(
                f,
                "{}:{}:{}: {}",
                self.file, self.line, self.column, self.message
            )
        }
    }
}

pub fn problem(file: &str, message: impl Into<String>) -> Problem {
    Problem {
        file: file.to_owned(),
        line: 0,
        column: 0,
        message: message.into(),
    }
}

/// A stage's WGSL, and where each part of it came from.
#[derive(Debug)]
pub struct Composed {
    pub text: String,
    /// (the file, or `<prelude>` / `<entries>`, its byte range in `text`).
    pub parts: Vec<(String, std::ops::Range<usize>)>,
}

impl Composed {
    /// The file, line and column (1-based) of a byte offset in `text`.
    pub fn locate(&self, offset: usize) -> (&str, usize, usize) {
        let (file, range) = self
            .parts
            .iter()
            .find(|(_, r)| r.contains(&offset))
            .or(self.parts.last())
            .expect("a composed stage has parts");
        let before = &self.text[range.start..offset.clamp(range.start, range.end)];
        let line = before.matches('\n').count() + 1;
        let column = before.len() - before.rfind('\n').map_or(0, |i| i + 1) + 1;
        (file, line, column)
    }
}

/// Validates `module`, parsed from `composed`, locating a refusal in the file it came from.
pub fn validate(composed: &Composed, module: &naga::Module) -> Result<ModuleInfo, Vec<Problem>> {
    Validator::new(ValidationFlags::all(), Capabilities::default())
        .validate(module)
        .map_err(|e| {
            let at = e.spans().next().and_then(|(span, _)| span.to_range());
            let mut message = e.as_inner().to_string();
            let mut source = std::error::Error::source(e.as_inner());
            while let Some(s) = source {
                message += &format!(": {s}");
                source = s.source();
            }
            vec![located(composed, at.map(|r| r.start), message)]
        })
}

pub fn located(composed: &Composed, offset: Option<usize>, message: String) -> Problem {
    match offset {
        Some(at) => {
            let (file, line, column) = composed.locate(at);
            Problem {
                file: file.to_owned(),
                line,
                column,
                message,
            }
        }
        None => problem(&composed.parts[composed.parts.len() - 2].0, message),
    }
}

/// Translates every entry point for Vulkan (SPIR-V), Metal and DirectX 12 (HLSL) with the
/// override values `constants`, as wgpu does when it builds the pipelines: each refusal as
/// `<backend> refuses it: <reason>`, those of the first entry point refused.
pub fn translate(
    module: &naga::Module,
    info: &ModuleInfo,
    constants: &naga::back::PipelineConstants,
) -> Vec<String> {
    let mut problems = Vec::new();
    for e in &module.entry_points {
        let fail = |backend: &str, err: String| format!("{backend} refuses it: {err}");
        let processed = naga::back::pipeline_constants::process_overrides(
            module,
            info,
            Some((e.stage, &e.name)),
            constants,
        );
        let (m, i) = match processed {
            Ok(p) => p,
            Err(err) => return vec![fail("the override step", err.to_string())],
        };
        let spv = refusal(|| {
            naga::back::spv::write_vec(
                &m,
                &i,
                &naga::back::spv::Options::default(),
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: e.stage,
                    entry_point: e.name.clone(),
                }),
            )
        });
        let msl = refusal(|| {
            naga::back::msl::write_string(
                &m,
                &i,
                &naga::back::msl::Options {
                    lang_version: (2, 2),
                    ..Default::default()
                },
                &naga::back::msl::PipelineOptions {
                    entry_point: Some((e.stage, e.name.clone())),
                    ..Default::default()
                },
            )
        });
        let hlsl = refusal(|| {
            naga::back::hlsl::Writer::new(
                &mut String::new(),
                &naga::back::hlsl::Options::default(),
                &naga::back::hlsl::PipelineOptions {
                    entry_point: Some((e.stage, e.name.clone())),
                },
            )
            .write(&m, &i, None)
        });
        let errors = [
            ("Vulkan (SPIR-V)", spv),
            ("Metal", msl),
            ("DirectX 12 (HLSL)", hlsl),
        ];
        for (backend, err) in errors {
            if let Some(err) = err {
                problems.push(fail(backend, err));
            }
        }
        if !problems.is_empty() {
            return problems;
        }
    }
    problems
}

/// Why a backend refused: its error, or the panic of a backend that cannot translate a
/// shader it was given (a mod's shader must never take the game or the tool down).
fn refusal<T, E: fmt::Display>(translate: impl FnOnce() -> Result<T, E>) -> Option<String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(translate)) {
        Ok(result) => result.err().map(|e| e.to_string()),
        Err(panic) => Some(format!(
            "it panicked: {}",
            panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or(panic.downcast_ref::<&str>().copied())
                .unwrap_or("no reason given")
        )),
    }
}
