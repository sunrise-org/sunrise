//! The core-shader mode (`--core`): vanilla's `assets/minecraft/shaders/core/` programs, read
//! from the prepared resources (`SUNRISE_TEST_RESOURCES`, never committed), mutated by the same
//! steps as a pack's programs, and run through `sunrise_glsl::translate` as the game runs a
//! resource pack's core shaders (`sunrise-client-render`'s `core_shader.rs`, which this crate
//! can't depend on: the tables below are its).
//!
//! A case's files are the programs' (`core/<name>.vsh` and `.fsh`) and their includes
//! (`include/*.glsl`), by path inside `shaders`, staged as a pack's are. The vertex inputs a
//! program's vertex shader declares are what its glue offers, as attributes in declaration
//! order: no pipeline is given, so a program the game translates with a glue of its own is
//! translated with the attributes it declares.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use sunrise_glsl::{Error, Glue, Includes, Inputs, Stages, Translated};

use crate::cases::{Entry, Source, Stream, global_points, is_ident, join, lines};
use crate::check::{self, Failure, Observed, Request};

/// Where vanilla's programs are in the staged pack (inside `shaders`), and their includes.
const FOLDER: &str = "core";
const INCLUDES: &str = "include";

/// The programs' folder as the ids of its files name it, and as `Stages::dir` gives it.
const DIR: &str = "minecraft:shaders/core/";

/// Vanilla's uniform blocks (`shaders/include/*.glsl`) and their bindings in set 0.
const BLOCKS: [(&str, u32); 4] = [
    ("Globals", 0),
    ("Projection", 1),
    ("DynamicTransforms", 2),
    ("Fog", 3),
];

/// Vanilla's samplers: the texture's binding in set 0, the sampler object's the next one.
const SAMPLERS: [(&str, u32); 3] = [("Sampler0", 4), ("Sampler1", 6), ("Sampler2", 8)];

/// The members of [`BLOCKS`] that packs still declare as loose uniforms.
const LOOSE: [&str; 18] = [
    "CameraBlockPos",
    "GlintAlpha",
    "CameraOffset",
    "GameTime",
    "ScreenSize",
    "MenuBlurRadius",
    "UseRgss",
    "ProjMat",
    "TextureMat",
    "ModelViewMat",
    "ColorModulator",
    "FogColor",
    "FogEnvironmentalStart",
    "FogRenderDistanceStart",
    "FogRenderDistanceEnd",
    "FogEnd",
    "FogStart",
    "FogShape",
];

/// What the game's compiler defines for every program on wgpu's 0 to 1 depth.
const DEFINES: [&str; 1] = ["RENDERPEARL_DEPTH_IS_ZERO_TO_ONE"];

/// The GLSL types of a vertex input.
const INPUT_TYPES: [&str; 12] = [
    "float", "vec2", "vec3", "vec4", "int", "ivec2", "ivec3", "ivec4", "uint", "uvec2", "uvec3",
    "uvec4",
];

type Read<'a> = &'a dyn Fn(&str) -> Result<Option<String>, String>;

/// The core programs under the resources `entry.path`: the files of `core/` and `include/`, and
/// the programs with both stages. Why they can't be read.
pub fn open(entry: &Entry) -> Result<Source, String> {
    let shaders = entry.path.join("assets/minecraft/shaders");
    let mut files = BTreeMap::new();
    for folder in [FOLDER, INCLUDES] {
        let at = shaders.join(folder);
        let listed = std::fs::read_dir(&at).map_err(|e| format!("{}: {e}", at.display()))?;
        for item in listed {
            let item = item.map_err(|e| format!("{}: {e}", at.display()))?;
            let (path, name) = (item.path(), item.file_name());
            let Some(name) = name.to_str() else {
                continue;
            };
            if path.is_file() {
                let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                files.insert(format!("{folder}/{name}"), bytes);
            }
        }
    }
    let programs: Vec<String> = files
        .keys()
        .filter_map(|path| path.strip_prefix("core/")?.strip_suffix(".fsh"))
        .filter(|name| files.contains_key(&format!("core/{name}.vsh")))
        .map(str::to_owned)
        .collect();
    Ok(Source {
        name: entry.name.clone(),
        pack: None,
        files,
        folders: [
            (FOLDER.to_owned(), programs),
            Default::default(),
            Default::default(),
        ],
    })
}

/// The text of the file `id` names (`minecraft:shaders/...`), as `file` answers a path inside
/// `shaders`: none when there is none, or `id` isn't a resource of the game's folder (a path
/// with an empty, `.` or `..` part is not one), or why it isn't text.
fn resolve(file: &dyn Fn(&str) -> Option<Vec<u8>>, id: &str) -> Result<Option<String>, String> {
    let Some(path) = id.strip_prefix("minecraft:shaders/") else {
        return Ok(None);
    };
    if path.split('/').any(|part| matches!(part, "" | "." | "..")) {
        return Ok(None);
    }
    match file(path) {
        None => Ok(None),
        Some(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| format!("{id}: not text")),
    }
}

/// Vanilla's include layout: `<path>` is `minecraft:shaders/include/path`, and `#moj_import`
/// pastes a file once.
fn includes(read: Read<'_>) -> Includes<'_> {
    Includes {
        read,
        namespace: "minecraft",
        folder: "shaders/include/",
        once: &["moj_import"],
    }
}

/// The files the stage files of `programs` read: each one, and each file its includes ask for
/// that the case holds.
pub fn reach(files: &BTreeMap<String, Vec<u8>>, programs: &[String]) -> BTreeSet<String> {
    let asked = RefCell::new(BTreeSet::new());
    let read = |id: &str| {
        asked.borrow_mut().insert(id.to_owned());
        resolve(&|path| files.get(path).cloned(), id)
    };
    for program in programs {
        for stage in ["vsh", "fsh"] {
            let id = format!("{DIR}{program}.{stage}");
            if let Ok(Some(text)) = read(&id) {
                let _ = sunrise_glsl::expand_includes(
                    &text,
                    &id,
                    DIR,
                    &includes(&read),
                    Some(&DEFINES[..]),
                );
            }
        }
    }
    asked
        .into_inner()
        .into_iter()
        .filter_map(|id| id.strip_prefix("minecraft:shaders/").map(str::to_owned))
        .filter(|path| files.contains_key(path))
        .collect()
}

/// The inputs `vertex` declares (`in vec3 Position;`, with or without a `layout(...)`), in
/// order, each name once.
fn vertex_inputs(vertex: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for line in vertex.lines() {
        let line = line.trim();
        let line = match line.strip_prefix("layout") {
            Some(rest) => rest
                .split_once(')')
                .map_or("", |(_, after)| after.trim_start()),
            None => line,
        };
        let Some((ty, name)) = line
            .strip_prefix("in ")
            .and_then(|rest| rest.trim().strip_suffix(';'))
            .and_then(|rest| rest.trim().split_once(' '))
        else {
            continue;
        };
        let name = name.trim();
        if INPUT_TYPES.contains(&ty) && is_ident(name) && found.iter().all(|(n, _)| n != name) {
            found.push((name.to_owned(), ty.to_owned()));
        }
    }
    found
}

/// The program `program` of `read`'s files through `sunrise_glsl::translate`: the blocks,
/// samplers and defines of the game's compiler, every other block or sampler given, and the
/// vertex inputs the vertex shader declares as attributes.
fn translate(read: Read<'_>, program: &str) -> Result<Translated, Vec<Error>> {
    let stage = |extension: &str| {
        let file = format!("{DIR}{program}.{extension}");
        let refused = |message: String| {
            vec![Error {
                file: file.clone(),
                line: 0,
                message,
            }]
        };
        match read(&file) {
            Ok(Some(text)) => Ok((file.clone(), text)),
            Ok(None) => Err(refused("not found".to_owned())),
            Err(why) => Err(refused(why)),
        }
    };
    let (vertex_file, vertex) = stage("vsh")?;
    let (fragment_file, fragment) = stage("fsh")?;
    let declared = vertex_inputs(&vertex);
    let names: Vec<(&str, &str)> = declared
        .iter()
        .map(|(name, ty)| (name.as_str(), ty.as_str()))
        .collect();
    let prelude = (declared.iter().enumerate())
        .map(|(at, (name, ty))| format!("layout(location = {at}) in {ty} hz_{name};"))
        .collect::<Vec<_>>()
        .join("\n");
    let setup = (declared.iter())
        .map(|(name, _)| format!("{name} = hz_{name};"))
        .collect::<Vec<_>>()
        .join(" ");
    let glue = Glue {
        inputs: &names,
        prelude: &prelude,
        setup: &setup,
        finish: "",
        vertex_id: "int(gl_VertexIndex)",
        fragment_finish: None,
        interface: sunrise_shaders::DEVICELESS_INTERFACE_LIMITS,
    };
    let includes = includes(read);
    let inputs = Inputs {
        blocks: &BLOCKS,
        samplers: &SAMPLERS,
        loose: &LOOSE,
        defines: &DEFINES,
        includes: &includes,
    };
    let sources = Stages {
        dir: DIR,
        vertex: &vertex,
        fragment: &fragment,
        vertex_file: &vertex_file,
        fragment_file: &fragment_file,
    };
    sunrise_glsl::translate(&sources, &inputs, &glue, &|_| true)
}

/// What the request's programs give as the pack staged in `dir` holds them: a program that
/// translates is loaded, its modules hashed and validated; one that doesn't has a note for each
/// error, naming the program, unless the error names no file, which a note must (`check::named`).
pub fn observe(dir: &Path, request: &Request) -> Observed {
    let read = |id: &str| {
        resolve(
            &|path| std::fs::read(dir.join("shaders").join(path)).ok(),
            id,
        )
    };
    let mut seen = Observed::default();
    for program in &request.programs {
        match translate(&read, program) {
            Ok(translated) => {
                seen.loaded.push(program.clone());
                for (stage, module) in [
                    ("vertex", &translated.vertex),
                    ("fragment", &translated.fragment),
                ] {
                    seen.pieces
                        .push((format!("{program} {stage}"), check::hash(module)));
                    if let Some(error) = check::validate(module) {
                        seen.invalid
                            .push((format!("core {program} {stage}"), error));
                    }
                }
                let bindings = (&translated.other, &translated.read);
                seen.pieces
                    .push((format!("{program} bindings"), check::hash(&bindings)));
            }
            Err(errors) => {
                for error in errors {
                    let text = error.to_string();
                    seen.notes.push(if check::named(&text) {
                        format!("{program}: {text}")
                    } else {
                        text
                    });
                }
            }
        }
    }
    seen.pieces
        .insert(0, ("notes".into(), check::hash(&seen.notes)));
    seen
}

/// Each stage file of the request's programs whose includes, expanded as the translator
/// expands them, grow past `request.size_limit`.
pub fn oversized(dir: &Path, request: &Request) -> Vec<Failure> {
    let read = |id: &str| {
        resolve(
            &|path| std::fs::read(dir.join("shaders").join(path)).ok(),
            id,
        )
    };
    let mut over = Vec::new();
    for program in &request.programs {
        for stage in ["vsh", "fsh"] {
            let id = format!("{DIR}{program}.{stage}");
            let Ok(Some(text)) = read(&id) else {
                continue;
            };
            let expanded = sunrise_glsl::expand_includes(
                &text,
                &id,
                DIR,
                &includes(&read),
                Some(&DEFINES[..]),
            );
            if expanded.is_ok_and(|e| e.text().len() > request.size_limit) {
                over.push(Failure::new(
                    "size",
                    format!("{program}.{stage}: preprocessed past the limit"),
                ));
            }
        }
    }
    over
}

/// A line of includes in `text`: chains past the translator's depth, cycles, diamonds, paths out
/// of the folder, files that aren't there, and a file pasted twice by `#moj_import`. Its files
/// are written into `files`, by the includes' folder.
pub fn include(
    s: &mut Stream,
    file: &str,
    text: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> String {
    let mut add = |name: &str, body: String| {
        files.insert(format!("{INCLUDES}/{name}"), body.into_bytes());
    };
    let line = match s.below(8) {
        0 => {
            let depth = *s.pick(&[2usize, 5, 9, 16, 17, 40]);
            for i in 0..depth {
                let body = if i + 1 < depth {
                    format!("#include <fz_inc{}.glsl>\n", i + 1)
                } else {
                    "float fz_chain() { return 1.0; }\n".to_owned()
                };
                add(&format!("fz_inc{i}.glsl"), body);
            }
            "#include <fz_inc0.glsl>".to_owned()
        }
        1 => format!("#include \"{}\"", file.rsplit('/').next().unwrap_or(file)),
        2 => {
            let back = *s.pick(&["include", "moj_import"]);
            add(
                "fz_a.glsl",
                "#include \"fz_b.glsl\"\nfloat fz_a1;\n".to_owned(),
            );
            add("fz_b.glsl", format!("#{back} <fz_a.glsl>\nfloat fz_b1;\n"));
            "#include <fz_a.glsl>".to_owned()
        }
        3 => {
            let levels = *s.pick(&[3usize, 6, 9, 12]);
            for i in 0..levels {
                let body = if i + 1 < levels {
                    format!("#include <fz_d{0}.glsl>\n#include <fz_d{0}.glsl>\n", i + 1)
                } else {
                    "#define FZ_LEAF 1\n// a leaf, included 2^levels times\n".to_owned()
                };
                add(&format!("fz_d{i}.glsl"), body);
            }
            "#include <fz_d0.glsl>".to_owned()
        }
        4 => (*s.pick(&[
            "#include <../../../../etc/hostname>",
            "#include <minecraft:../core/text.vsh>",
            "#include <>",
            "#include <minecraft:>",
            "#include <other:fz.glsl>",
            "#include <fz.glsl",
            "#include \"\"",
            "#include \"../core/text.vsh\"",
            "#include \"/include/fog.glsl\"",
            "#include",
            "#moj_import",
            "#moj_import <>",
            "#moj_import \"fz.glsl\"",
        ]))
        .to_owned(),
        5 => format!("#{} <fz_missing.glsl>", *s.pick(&["include", "moj_import"])),
        6 => {
            add("fz_once.glsl", "const float fz_once = 2.0;\n".to_owned());
            "#moj_import <fz_once.glsl>\n#moj_import <fz_once.glsl>".to_owned()
        }
        _ => {
            // A file of the folder that is already in the case, a second time.
            let others: Vec<String> = (files.keys())
                .filter_map(|f| f.strip_prefix("include/").map(str::to_owned))
                .collect();
            if others.is_empty() {
                "#include <fz_missing.glsl>".to_owned()
            } else if s.chance(2) {
                format!("#include <{}>", s.pick(&others))
            } else {
                format!("#moj_import <minecraft:{}>", s.pick(&others))
            }
        }
    };
    let mut ls = lines(text);
    let points = global_points(&ls);
    let at = if s.chance(4) {
        s.below(ls.len() + 1)
    } else {
        *s.pick(&points)
    };
    ls.insert(at.min(ls.len()), line);
    join(&ls)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cases::{CORE_KINDS, Case, Mode, case_seed, core_entry, generate};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Resources with the programs `files` (paths under `assets/minecraft/shaders`).
    fn resources(files: &[(&str, &str)]) -> std::path::PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "shader-fuzz-core-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in files {
            let file = root.join("assets/minecraft/shaders").join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        root
    }

    const VERTEX: &str = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\n\
        layout(location = 2) in vec2 UV0;\nlayout(location = 3) in ivec2 UV2;\n\
        layout(location = 0) out vec4 vertexColor;\nlayout(location = 1) out vec2 texCoord0;\n\
        void main() { gl_Position = vec4(Position, 1.0); vertexColor = Color; texCoord0 = UV0; }\n";
    const FRAGMENT: &str = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        layout(location = 0) in vec4 vertexColor;\nlayout(location = 1) in vec2 texCoord0;\n\
        layout(location = 0) out vec4 fragColor;\n\
        void main() { if (texCoord0.x < 0.5) discard; fragColor = vertexColor; }\n";

    fn request(programs: &[&str]) -> Request {
        Request {
            dim: 0,
            flat: true,
            mode: Mode::Core,
            programs: programs.iter().map(|p| (*p).to_owned()).collect(),
            expect: Vec::new(),
            size_limit: 16 << 20,
        }
    }

    #[test]
    fn a_vertex_shader_declares_the_inputs_its_glue_offers() {
        let vertex = "#version 330\nlayout(location = 0) in vec3 Position;\nin vec4 Color;\n\
            in vec2 UV0[2];\nout vec4 x;\nin float a b;\nin vec3 Position;\nin sampler2D S;\n";
        assert_eq!(
            vertex_inputs(vertex),
            [
                ("Position".to_owned(), "vec3".to_owned()),
                ("Color".to_owned(), "vec4".to_owned())
            ]
        );
    }

    #[test]
    fn a_program_that_translates_is_loaded_and_one_that_doesnt_is_named() {
        let root = resources(&[]);
        let shaders = root.join("shaders/core");
        std::fs::create_dir_all(&shaders).unwrap();
        std::fs::write(shaders.join("good.vsh"), VERTEX).unwrap();
        std::fs::write(shaders.join("good.fsh"), FRAGMENT).unwrap();
        std::fs::write(shaders.join("bad.vsh"), VERTEX).unwrap();
        std::fs::write(shaders.join("bad.fsh"), "void main() {\n").unwrap();
        let seen = observe(&root, &request(&["bad", "good"]));
        assert_eq!(seen.loaded, ["good"]);
        assert!(!seen.notes.is_empty());
        assert_eq!(seen.invalid, []);
        let outcome = check::check(&root, &request(&["good"]));
        assert_eq!(outcome.failures, []);
        assert_eq!(outcome.loaded, ["good"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Cases are made from a seed alone: the same steps, over the files of vanilla's folders.
    #[test]
    fn a_core_case_is_seeded_and_takes_only_core_steps() {
        let root = resources(&[
            ("core/p.vsh", VERTEX),
            ("core/p.fsh", FRAGMENT),
            ("core/q.vsh", VERTEX),
            ("core/q.fsh", FRAGMENT),
            ("core/lonely.vsh", VERTEX),
            ("include/fog.glsl", "float fog_value;\n"),
        ]);
        let index = [core_entry(&root)];
        let mut steps = 0;
        for i in 0..40 {
            let seed = case_seed(5, i);
            let case: Case = generate(&index, seed, true).unwrap();
            assert_eq!(Ok(&case), generate(&index, seed, true).as_ref());
            assert_eq!(case.mode, Mode::Core);
            assert!(case.programs.iter().all(|p| p == "p" || p == "q"));
            assert!(case.steps.iter().all(|s| CORE_KINDS.contains(&s.kind)));
            steps += case.steps.len();
        }
        assert!(steps >= 40);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_include_step_is_seeded_and_writes_what_it_names() {
        let text = "#version 330\nvoid main() {}\n";
        let made = |seed: u64| {
            let mut files = BTreeMap::new();
            let out = include(&mut Stream(seed), "core/p.vsh", text, &mut files);
            (out, files)
        };
        for seed in 0..64 {
            let (out, files) = made(seed);
            assert_eq!((out.clone(), files), made(seed));
            assert!(
                out.contains("#include") || out.contains("#moj_import"),
                "{out}"
            );
        }
        let chain = (0..256)
            .map(made)
            .find(|(out, _)| out.contains("<fz_inc0.glsl>"));
        let (_, files) = chain.expect("a seed that writes a chain");
        assert!(files.contains_key("include/fz_inc0.glsl"));
    }
}
