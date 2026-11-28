//! A program with a geometry stage (`shadow.gsh`), which wgpu lacks: its vertex, geometry and
//! fragment stages become one vertex stage and the fragment stage. The game draws its triangles
//! without an index buffer, three vertices each, and one instance per triangle the geometry stage
//! can emit: vertex invocation `v` of instance `t` stands for corner `v % 3` of the stage's
//! triangle `t`. It reads its input triangle's three vertices itself (`GeometryInputs::index`,
//! `pull`), runs the pack's vertex stage on each, then the geometry stage, keeping the corner it
//! stands for; a corner the stage does not emit is degenerate.
//!
//! The geometry stage runs once per output corner; the image and buffer writes it makes as
//! statements run in one of them only (`hz_primary`).

use std::collections::BTreeSet;
use std::fmt::Write;

use super::scan::{declaration_prefix, significant};
use super::{Error, GbuffersInputs, Program, Target};
use crate::Preprocessed;
use crate::macros::lex;
use crate::preprocess::check_line_count;

/// How a geometry program's vertex invocations reach their triangle's vertices.
#[derive(Clone, Debug, Default)]
pub struct GeometryInputs {
    /// The game's vertex inputs, as `GbuffersInputs`; their `layout(location = n) in`
    /// declarations become plain globals the pull sets.
    pub inputs: GbuffersInputs,
    /// GLSL statements that set the vertex inputs from vertex `hz_vertex_index` (a `uint`).
    pub pull: String,
    /// The vertex of the `@`th corner of the draw (a `uint` expression, `hz_indices[@]`).
    pub index: String,
}

/// Translates `vertex`, `geometry` and `fragment` (a shadow program): the program, and how many
/// triangles the geometry stage can emit (the instances to draw). A fold that cannot keep a
/// stage's line origins is refused at that stage's file.
pub fn translate_geometry(
    vertex: &Preprocessed,
    geometry: &Preprocessed,
    fragment: &Preprocessed,
    inputs: &GeometryInputs,
    limits: naga::front::glsl::InterfaceLimits,
) -> Result<(Program, u32), Vec<Error>> {
    super::guard_stage(vertex, naga::ShaderStage::Vertex).map_err(|e| vec![e])?;
    super::guard_stage(geometry, naga::ShaderStage::Vertex).map_err(|e| vec![e])?;
    super::guard_stage(fragment, naga::ShaderStage::Fragment).map_err(|e| vec![e])?;
    let (combined, triangles) = combine(vertex, geometry, inputs)?;
    let mut gbuffers = inputs.inputs.clone();
    gbuffers.declarations = gbuffers
        .declarations
        .lines()
        .map(|l| match l.find(") in ") {
            Some(at) if l.starts_with("layout(location") => &l[at + 5..],
            _ => l,
        })
        .fold(String::new(), |s, l| s + l + "\n");
    // The pack's vertex stage reads its inputs in our loop, not in `main`.
    gbuffers.body = String::new();
    // The vertex's place isn't the fragment's through a geometry stage.
    gbuffers.water_patches = false;
    gbuffers.attributes = Vec::new();
    let program = super::translate_inner(
        &combined,
        fragment,
        Target::Buffers,
        Some(&gbuffers),
        true,
        limits,
    )?;
    Ok((program, triangles))
}

/// A global statement's tokens and what it declares.
struct Statement {
    range: std::ops::Range<usize>,
    kind: Kind,
}

enum Kind {
    /// `in`, `out`, `attribute`, `varying`: storage, names and types (with array suffixes).
    Interface(&'static str, Vec<(String, String)>),
    /// An unflattened `in`/`out` interface block, left in place so its authored lines stay intact.
    InterfaceBlock,
    /// `layout(...) in;` or `out;` without a variable.
    Layout(&'static str),
    Buffer(String),
    Uniform,
    /// Functions, variables, structs: the names it defines.
    Defines(Vec<String>),
}

fn statements(tokens: &[&str]) -> Vec<Statement> {
    let mut out = Vec::new();
    let (mut start, mut parens) = (0, 0usize);
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "(" => parens += 1,
            ")" => parens = parens.saturating_sub(1),
            "{" if parens == 0 => {
                // A function body, a struct or a block: to its matching `}` (and `;`).
                let mut d = 0;
                let mut j = i;
                while j < tokens.len() {
                    match tokens[j] {
                        "{" => d += 1,
                        "}" => {
                            d -= 1;
                            if d == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                let mut end = j + 1;
                let head: Vec<&str> = tokens[start..i]
                    .iter()
                    .copied()
                    .filter(|t| significant(t))
                    .collect();
                // `struct S { } ;`, `buffer B { } name;`, `layout(...) out B { } name;`: to `;`.
                // Only a function body's `{` immediately follows its parameter list's `)`.
                if head.last() != Some(&")") {
                    while end < tokens.len() && tokens[end] != ";" {
                        end += 1;
                    }
                    end += 1;
                }
                out.push(classify(tokens, start..end.min(tokens.len())));
                start = end.min(tokens.len());
                i = start;
                continue;
            }
            ";" if parens == 0 => {
                out.push(classify(tokens, start..i + 1));
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The tokens among `tokens` that declare a member of a struct or a block (`vaNormal` in
/// `struct S { vec3 vaNormal; };` or `buffer B { vec3 vaNormal; } params;`), at file scope or
/// in a function: those of each body whose `{` follows a name or `struct` (not a function's,
/// after `)`, nor a statement's, nor an initializer list's), read as the scanner reads members
/// (`scan::members`). A member keeps its name, as each access to it (`params.vaNormal`) does;
/// a block without an instance makes its members globals of the stage, which the scan
/// declares, so they aren't renamed either.
fn members(tokens: &[&str]) -> BTreeSet<usize> {
    let mut members = BTreeSet::new();
    for (open, &t) in tokens.iter().enumerate() {
        if t != "{" {
            continue;
        }
        let prev = (0..open).rev().find(|&j| significant(tokens[j]));
        if prev.is_some_and(|j| {
            super::scan::is_ident(tokens[j]) && !matches!(tokens[j], "else" | "do")
        }) {
            members.extend(super::scan::members(tokens, open).0);
        }
    }
    members
}

fn classify(tokens: &[&str], range: std::ops::Range<usize>) -> Statement {
    let words: Vec<&str> = tokens[range.clone()]
        .iter()
        .copied()
        .filter(|t| significant(t))
        .collect();
    let (storage, k) = declaration_prefix(&words);
    let rest = &words[k..];
    let kind = match storage {
        Some("buffer") => Kind::Buffer(rest.first().map_or(String::new(), |s| (*s).to_owned())),
        Some("uniform") => Kind::Uniform,
        Some(storage) => match rest.first() {
            None | Some(&";") => Kind::Layout(storage),
            Some(_) if rest.contains(&"{") => Kind::InterfaceBlock,
            Some(ty) => {
                let mut vars = Vec::new();
                let mut j = 1;
                while j < rest.len() && rest[j] != ";" {
                    let name = rest[j].to_owned();
                    let mut array = String::new();
                    j += 1;
                    while j < rest.len() && !matches!(rest[j], "," | ";") {
                        array.push_str(rest[j]);
                        j += 1;
                    }
                    j += 1;
                    vars.push((name, format!("{ty}{array}")));
                }
                Kind::Interface(storage, vars)
            }
        },
        None => {
            let mut names = Vec::new();
            let first = rest.iter().position(|w| matches!(*w, "(" | "="));
            // A struct's `struct` comes before its body; a function's body may define structs.
            let head = rest.iter().position(|w| matches!(*w, "(" | "{"));
            if let Some(p) = rest[..head.unwrap_or(rest.len())]
                .iter()
                .position(|w| *w == "struct")
            {
                names.extend(rest.get(p + 1).map(|s| (*s).to_owned()));
            } else if let Some(p) = first.filter(|&p| rest[p] == "(") {
                // A function.
                names.extend(p.checked_sub(1).map(|n| rest[n].to_owned()));
            } else {
                // Variables: the last identifier before each declarator's `=` or end.
                let (mut depth, mut last, mut init) = (0, None, false);
                for w in rest {
                    match *w {
                        "(" | "[" | "{" => depth += 1,
                        ")" | "]" | "}" => depth -= 1,
                        "=" if depth == 0 => {
                            names.extend(last.take());
                            init = true;
                        }
                        "," | ";" if depth == 0 => {
                            if !init {
                                names.extend(last.take());
                            }
                            (last, init) = (None, false);
                        }
                        w if depth == 0 && !init && super::scan::is_ident(w) => {
                            last = Some((*w).to_owned())
                        }
                        _ => {}
                    }
                }
            }
            names.dedup();
            Kind::Defines(names)
        }
    };
    Statement { range, kind }
}

/// The combined vertex stage's source, and the triangles the geometry stage can emit.
/// Each rewritten stage must still match its own source map before it can join the fold.
fn combine(
    vertex: &Preprocessed,
    geometry: &Preprocessed,
    inputs: &GeometryInputs,
) -> Result<(Preprocessed, u32), Vec<Error>> {
    let error = |message: String| vec![Error::new(geometry.file(), 0, message)];
    // Interface blocks first become plain varyings (`interface_blocks`), the vertex stage's
    // `out` block and the geometry stage's `in` array alike: what follows reads declarations
    // of one type each. The geometry stage's input blocks read as `hz_in_<block>_<member>`,
    // apart from its output blocks, which may have the same names.
    let gs_code = super::interface_blocks::flatten_as(geometry.code(), "in_");
    let vs_code = super::interface_blocks::flatten(vertex.code());
    let gs_tokens = lex(gs_code.as_deref().unwrap_or(geometry.code()));
    let vs_tokens = lex(vs_code.as_deref().unwrap_or(vertex.code()));
    let gs = statements(&gs_tokens);
    let vs = statements(&vs_tokens);

    // The geometry stage's interface.
    let mut gs_ins: Vec<(String, String)> = Vec::new();
    let mut gs_outs: Vec<(String, String)> = Vec::new();
    let mut gs_buffers = BTreeSet::new();
    let mut max_vertices = None;
    let mut gs_text: Vec<String> = gs_tokens.iter().map(|t| t.to_string()).collect();
    for s in &gs {
        match &s.kind {
            Kind::Interface("in", vars) => {
                // The input primitive is a triangle: `in vec3 v[]` has three elements.
                let vars: Vec<(String, String)> = vars
                    .iter()
                    .map(|(n, ty)| (n.clone(), ty.replace("[]", "[3]")))
                    .collect();
                gs_ins.extend(vars.iter().cloned());
                // One declaration per name (naga types `T a[3], b[3]`'s second wrong).
                let decls: String = vars
                    .iter()
                    .map(|(n, ty)| {
                        let (base, array) = split_type(ty);
                        format!("{base} {n}{array}; ")
                    })
                    .collect();
                blank(&mut gs_text, s.range.clone());
                gs_text[s.range.start] = decls + &gs_text[s.range.start];
            }
            Kind::Interface("out", vars) => gs_outs.extend(vars.iter().cloned()),
            Kind::Layout(storage) => {
                if *storage == "out" {
                    let words: Vec<&str> = gs_tokens[s.range.clone()]
                        .iter()
                        .copied()
                        .filter(|t| significant(t))
                        .collect();
                    if let Some(p) = words.iter().position(|w| *w == "max_vertices") {
                        max_vertices = words.get(p + 2).and_then(|n| n.parse::<u32>().ok());
                    }
                }
                blank(&mut gs_text, s.range.clone());
            }
            Kind::Buffer(name) => _ = gs_buffers.insert(name.clone()),
            _ => {}
        }
    }
    let Some(max_vertices) = max_vertices else {
        return Err(error("the geometry stage has no `max_vertices`".into()));
    };
    for (i, t) in gs_tokens.iter().enumerate() {
        let prev = (0..i).rev().find(|&j| significant(gs_tokens[j]));
        let next = (i + 1..gs_tokens.len()).find(|&j| significant(gs_tokens[j]));
        let called = next.is_some_and(|j| gs_tokens[j] == "(");
        match *t {
            "EmitVertex" if called => gs_text[i] = "hz_emit".into(),
            "EndPrimitive" if called => gs_text[i] = "hz_end".into(),
            "gl_in" => gs_text[i] = "hz_gl_in".into(),
            "gl_Position" if prev.is_some_and(|j| gs_tokens[j] == ".") => gs_text[i] = "p".into(),
            "main" if called => gs_text[i] = "hz_gs_main".into(),
            _ => {}
        }
    }

    // The vertex stage: its globals renamed, its outputs plain globals, its buffer blocks the
    // geometry stage's.
    let mut defined: BTreeSet<String> = BTreeSet::new();
    let mut vs_outs: Vec<(String, String)> = Vec::new();
    let mut vs_attributes: Vec<(String, String)> = Vec::new();
    let mut vs_text: Vec<String> = vs_tokens.iter().map(|t| t.to_string()).collect();
    for s in &vs {
        match &s.kind {
            Kind::Defines(names) => defined.extend(names.iter().cloned()),
            Kind::Interface("out" | "varying", vars) => {
                // Declared again, renamed, in our head.
                vs_outs.extend(vars.iter().cloned());
                blank(&mut vs_text, s.range.clone());
            }
            Kind::Interface("in" | "attribute", vars) => {
                vs_attributes.extend(vars.iter().cloned());
                strip_storage(&mut vs_text, &vs_tokens, s.range.clone());
            }
            Kind::Buffer(name) if gs_buffers.contains(name) => blank(&mut vs_text, s.range.clone()),
            Kind::Layout(_) => blank(&mut vs_text, s.range.clone()),
            _ => {}
        }
    }
    // The game inputs the vertex stage reads without declaring them (terrain's fade, a
    // `vaPosition`...) are set for each pulled source vertex, alongside declared attributes,
    // under the vertex stage's own names, apart from the geometry stage's globals.
    let implicit = super::implicit_inputs(
        &inputs.inputs.attributes,
        vertex,
        &super::scan::Stage::scan(vertex, naga::ShaderStage::Vertex),
    );
    defined.extend(implicit.iter().map(|(name, _, _)| name.to_string()));
    let renamed: BTreeSet<&str> = defined
        .iter()
        .map(String::as_str)
        .chain(vs_outs.iter().map(|(n, _)| n.as_str()))
        .collect();
    let members = members(&vs_tokens);
    for (i, t) in vs_tokens.iter().enumerate() {
        if vs_text[i] != *t || members.contains(&i) {
            continue;
        }
        let prev = (0..i).rev().find(|&j| significant(vs_tokens[j]));
        if prev.is_some_and(|j| vs_tokens[j] == ".") {
            continue;
        }
        if *t == "gl_Position" {
            vs_text[i] = "hz_vs_Position".into();
        } else if matches!(*t, "gl_VertexID" | "gl_VertexIndex") {
            // The vertex the pack's vertex stage runs for, not this invocation.
            vs_text[i] = "hz_vs_vertex".into();
        } else if matches!(*t, "gl_InstanceID" | "gl_InstanceIndex") {
            // The game draws terrain uninstanced; this invocation's instance is a triangle.
            vs_text[i] = "0".into();
        } else if renamed.contains(t) {
            vs_text[i] = format!("hz_vs_{t}");
        }
    }

    once(&mut gs_text, &gs_tokens);
    once(&mut vs_text, &vs_tokens);

    // Ours: before the geometry stage, its emission; after the vertex stage, `main`.
    let mut head = String::from(
        "void hz_emit(); void hz_end(); struct hz_PerVertex { vec4 p; }; hz_PerVertex hz_gl_in[3];\n\
         int hz_n; int hz_strip; int hz_tri; int hz_want_tri; int hz_want_corner; bool hz_found; bool hz_primary;\n\
         vec4 hz_ring_Position[3]; vec4 hz_res_Position; vec4 hz_vs_Position; int hz_vs_vertex;\n",
    );
    for (name, _, ty) in &implicit {
        writeln!(head, "{ty} hz_vs_{name};").unwrap();
    }
    for (name, ty) in &vs_outs {
        let (base, array) = split_type(ty);
        writeln!(head, "{base} hz_vs_{name}{array};").unwrap();
    }
    let mut emit = String::from(
        "void hz_emit() {\n    int hz_s = hz_n % 3;\n    hz_ring_Position[hz_s] = gl_Position;\n",
    );
    let mut take = String::from("        hz_res_Position = hz_ring_Position[hz_r];\n");
    let mut result = String::from("    gl_Position = hz_found ? hz_res_Position : vec4(0.0);\n");
    for (name, ty) in &gs_outs {
        let (base, array) = split_type(ty);
        if !array.is_empty() {
            return Err(error(format!(
                "`{name}`: array outputs of a geometry stage are not supported yet"
            )));
        }
        writeln!(head, "{base} hz_ring_{name}[3]; {base} hz_res_{name};").unwrap();
        writeln!(emit, "    hz_ring_{name}[hz_s] = {name};").unwrap();
        writeln!(take, "        hz_res_{name} = hz_ring_{name}[hz_r];").unwrap();
        writeln!(result, "    {name} = hz_res_{name};").unwrap();
    }
    write!(
        emit,
        "    hz_n++;\n    hz_strip++;\n    if (hz_strip >= 3) {{\n        if (hz_tri == hz_want_tri) {{\n            \
         int hz_k = hz_want_corner;\n            if (((hz_strip - 3) & 1) == 1 && hz_k < 2) {{ hz_k = 1 - hz_k; }}\n            \
         int hz_r = (hz_n - 3 + hz_k) % 3;\n{take}            hz_found = true;\n        }}\n        hz_tri++;\n    }}\n}}\n\
         void hz_end() {{ hz_strip = 0; }}\n"
    )
    .unwrap();
    let body = inputs
        .inputs
        .body
        .replace("gl_VertexIndex", "hz_vertex_index");
    let mut main = String::from(
        "void main() {\n    uint hz_v = uint(gl_VertexIndex);\n    uint hz_first = hz_v / 3u * 3u;\n    \
         hz_want_corner = int(hz_v % 3u);\n    hz_want_tri = int(gl_InstanceIndex);\n    \
         hz_primary = hz_want_tri == 0 && hz_want_corner == 0;\n    \
         for (int hz_i = 0; hz_i < 3; hz_i++) {\n",
    );
    writeln!(
        main,
        "        uint hz_vertex_index = {};",
        inputs.index.replace('@', "hz_first + uint(hz_i)")
    )
    .unwrap();
    main.push_str("        hz_vs_vertex = int(hz_vertex_index);\n");
    main.push_str(&inputs.pull);
    main.push_str(&body);
    for (name, value, ty) in &implicit {
        writeln!(main, "        hz_vs_{name} = {ty}({value});").unwrap();
    }
    for (name, ty) in &vs_attributes {
        let value = inputs
            .inputs
            .attributes
            .iter()
            .find(|a| &a.0 == name)
            .map_or_else(|| format!("{ty}(0)"), |a| format!("{ty}({})", a.1));
        writeln!(main, "        {name} = {value};").unwrap();
    }
    main.push_str("        hz_vs_main();\n");
    for (name, _) in &vs_outs {
        let block = name.strip_prefix("hz_").map(|m| format!("hz_in_{m}"));
        if let Some((input, _)) = gs_ins
            .iter()
            .find(|(n, _)| n == name || Some(n) == block.as_ref())
        {
            writeln!(main, "        {input}[hz_i] = hz_vs_{name};").unwrap();
        }
    }
    main.push_str("        hz_gl_in[hz_i].p = hz_vs_Position;\n    }\n    hz_gs_main();\n");
    main.push_str(&result);
    main.push_str("}\n");

    let gs_text = gs_text.concat();
    let vs_text = vs_text.concat();
    let out = join_stages(vertex, &vs_text, geometry, &gs_text, &head, &emit, &main)?;
    // A strip of n vertices is n - 2 triangles: at most `max_vertices - 2`, whichever of the
    // stage's paths runs.
    let triangles = max_vertices.saturating_sub(2).max(1);
    Ok((out, triangles))
}

/// Joins the rewritten geometry and vertex stages with our generated glue, checking each stage's
/// line count against its own source map so a dropped line in one cannot cancel an added line in
/// the other.
fn join_stages(
    vertex: &Preprocessed,
    vs_text: &str,
    geometry: &Preprocessed,
    gs_text: &str,
    head: &str,
    emit: &str,
    main: &str,
) -> Result<Preprocessed, Vec<Error>> {
    let error = |message: String| vec![Error::new(geometry.file(), 0, message)];
    let (mut code, mut lines, mut files) = (String::new(), Vec::new(), Vec::new());
    let ours = |code: &mut String, lines: &mut Vec<crate::Origin>, text: &str| {
        for l in text.lines() {
            code.push_str(l);
            code.push('\n');
            lines.push(crate::Origin::default());
        }
    };
    for (kind, stage, text, before) in [
        ("geometry", geometry, gs_text, head),
        ("vertex", vertex, vs_text, emit),
    ] {
        check_line_count(text, stage.lines().len()).map_err(|message| {
            vec![Error::new(
                stage.file(),
                0,
                format!("{kind} stage: {message}"),
            )]
        })?;
        let offset = files.len();
        files.extend_from_slice(stage.files());
        ours(&mut code, &mut lines, before);
        code.push_str(text);
        code.push('\n');
        lines.extend(stage.lines().iter().map(|o| crate::Origin {
            file: o.file + offset,
            line: o.line,
        }));
    }
    ours(&mut code, &mut lines, main);
    // The final `}` line needs no separator after it.
    code.pop();
    let mut out = Preprocessed::new(code, lines, files).map_err(error)?;
    out.version = geometry.version.clone().or(vertex.version.clone());
    Ok(out)
}

/// Functions whose calls write memory other invocations see.
fn writes(name: &str) -> bool {
    name == "imageStore" || name.starts_with("imageAtomic") || name.starts_with("atomic")
}

/// Calls that write images or buffers run for one invocation of a triangle only: as statements,
/// `if (!hz_primary) {} else imageStore(...)`; in an expression the others read the value instead,
/// `(hz_primary ? imageAtomicOr(i, p, v) : imageLoad(i, p).x)`, `(hz_primary ? atomicAdd(m, v) : m)`.
/// Any line breaks in copied arguments stay only in the original call.
fn once(text: &mut [String], tokens: &[&str]) {
    for (i, &t) in tokens.iter().enumerate() {
        let open = (i + 1..tokens.len()).find(|&j| significant(tokens[j]));
        let Some(open) = open.filter(|&j| tokens[j] == "(") else {
            continue;
        };
        if !writes(t) || text[i] != t {
            continue;
        }
        let prev = (0..i).rev().find(|&j| significant(tokens[j]));
        if prev.is_none_or(|j| matches!(tokens[j], ";" | "{" | "}" | ")" | "else")) {
            // A whole if/else, so that an `else` after the statement stays the pack's.
            text[i] = format!("if (!hz_primary) {{}} else {t}");
            continue;
        }
        // The call's arguments, at its top-level commas.
        let (mut depth, mut args, mut from, mut close) = (0, Vec::new(), open + 1, None);
        for (j, &u) in tokens.iter().enumerate().skip(open) {
            match u {
                "(" | "[" => depth += 1,
                ")" | "]" if depth == 1 && u == ")" => {
                    args.push(tokens[from..j].concat());
                    close = Some(j);
                    break;
                }
                ")" | "]" => depth -= 1,
                "," if depth == 1 => {
                    args.push(tokens[from..j].concat());
                    from = j + 1;
                }
                _ => {}
            }
        }
        let Some(close) = close else { continue };
        let read = if t.starts_with("imageAtomic") && args.len() >= 2 {
            format!("imageLoad({}, {}).x", args[0], args[1])
        } else if t.starts_with("atomic") && !args.is_empty() {
            format!("({})", args[0])
        } else {
            continue;
        };
        text[i] = format!("(hz_primary ? {t}");
        text[close] = format!(") : {})", read.replace('\n', " "));
    }
}

/// `vec3[2]` or `vec3` → (`vec3`, `[2]`).
fn split_type(ty: &str) -> (&str, &str) {
    match ty.find('[') {
        Some(at) => (&ty[..at], &ty[at..]),
        None => (ty, ""),
    }
}

/// Blanks a statement, keeping its new lines.
fn blank(text: &mut [String], range: std::ops::Range<usize>) {
    for t in &mut text[range] {
        *t = t.chars().filter(|&c| c == '\n').collect();
    }
}

/// Removes a declaration's `layout(...)`, storage and interpolation words to make a plain
/// global without moving later lines.
fn strip_storage(text: &mut [String], tokens: &[&str], range: std::ops::Range<usize>) {
    let mut in_layout = false;
    for i in range {
        match tokens[i] {
            "layout" => in_layout = true,
            ")" if in_layout => {
                in_layout = false;
                text[i] = " ".into();
                continue;
            }
            _ => {}
        }
        if in_layout
            || matches!(
                tokens[i],
                "in" | "out"
                    | "attribute"
                    | "varying"
                    | "flat"
                    | "smooth"
                    | "noperspective"
                    | "centroid"
                    | "invariant"
            )
        {
            let breaks = tokens[i].matches('\n').count();
            text[i] = if breaks == 0 {
                " ".into()
            } else {
                "\n".repeat(breaks)
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::zip;
    use crate::{Macros, Pack, preprocess};

    /// A geometry stage that passes each triangle through with the vertex stage's `vNormal`.
    const PASS_GSH: &str = "#version 430\nlayout(triangles) in;\n\
                            layout(triangle_strip, max_vertices = 3) out;\n\
                            in vec3 vNormal[];\nout vec3 gNormal;\nvoid main() {\n    \
                            for (int i = 0; i < 3; i++) {\n        gNormal = vNormal[i];\n        \
                            gl_Position = gl_in[i].gl_Position;\n        EmitVertex();\n    }\n    \
                            EndPrimitive();\n}\n";
    /// The fragment stage after [`PASS_GSH`].
    const PASS_FSH: &str = "#version 430\nin vec3 gNormal;\nvoid main() {\n    gl_FragData[0] = vec4(gNormal, 1.0);\n}\n";

    /// `vsh`, `gsh` and `fsh` (a pack's shadow program) folded and translated.
    fn fold(vsh: &str, gsh: &str, fsh: &str) -> Result<(Program, u32), Vec<Error>> {
        fold_given(vsh, gsh, fsh, Vec::new())
    }

    /// `fold`, the game giving `attributes` (`GbuffersInputs::attributes`).
    fn fold_given(
        vsh: &str,
        gsh: &str,
        fsh: &str,
        attributes: Vec<(String, String)>,
    ) -> Result<(Program, u32), Vec<Error>> {
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                ("shaders/shadow.vsh", vsh),
                ("shaders/shadow.gsh", gsh),
                ("shaders/shadow.fsh", fsh),
            ]),
        )
        .unwrap();
        let pp = |file: &str| preprocess(&pack, file, &Macros::standard());
        let inputs = GeometryInputs {
            inputs: GbuffersInputs {
                declarations: "layout(set = 3, binding = 1) readonly buffer HzVertices { uint hz_vertices[]; };\n\
                               layout(location = 0) in vec3 hz_v_pos;"
                    .into(),
                body: "    hz_Vertex = vec4(hz_v_pos, 1.0);\n    hz_Color = vec4(1.0);\n".into(),
                attributes,
                ..Default::default()
            },
            pull: "        hz_v_pos = uintBitsToFloat(uvec3(hz_vertices[hz_vertex_index * 3u], \
                   hz_vertices[hz_vertex_index * 3u + 1u], hz_vertices[hz_vertex_index * 3u + 2u]));\n"
                .into(),
            index: "uint(@)".into(),
        };
        translate_geometry(
            &pp("shadow.vsh"),
            &pp("shadow.gsh"),
            &pp("shadow.fsh"),
            &inputs,
            naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
        )
    }

    #[test]
    fn a_geometry_stage_folds_into_the_vertex_stage() {
        let vsh = "#version 430\nout vec3 vPos;\nout vec4 vColor;\nvoid main() {\n    \
                   vPos = gl_Vertex.xyz;\n    vColor = gl_Color * float(gl_InstanceID + 1);\n    \
                   gl_Position = ftransform();\n}\n";
        let gsh = "#version 430\nlayout(triangles) in;\nlayout(triangle_strip, max_vertices = 4) out;\n\
                   in vec3 vPos[];\nin vec4 vColor[];\nout vec4 gColor;\nvoid main() {\n    \
                   for (int i = 0; i < 3; i++) {\n        gColor = vColor[i];\n        \
                   gl_Position = gl_in[i].gl_Position;\n        EmitVertex();\n    }\n    \
                   gl_Position = vec4(vPos[0], 1.0);\n    EmitVertex();\n    EndPrimitive();\n}\n";
        let fsh = "#version 430\nin vec4 gColor;\nvoid main() {\n    gl_FragData[0] = gColor;\n}\n";
        let (program, triangles) = fold(vsh, gsh, fsh).unwrap_or_else(|e| panic!("{e:?}"));
        // A strip of four corners is two triangles.
        assert_eq!(triangles, 2);
        assert_eq!(program.vertex.entry_points.len(), 1);
        assert_eq!(program.fragment.entry_points.len(), 1);
    }

    /// A vertex stage under a geometry stage reads the game's `vaPosition` and `vaNormal`
    /// without declaring them, as one without a geometry stage does: they're set for each
    /// pulled vertex. Without either one, the stage is refused.
    #[test]
    fn a_vertex_stage_under_a_geometry_stage_reads_undeclared_game_inputs() {
        let vsh = "#version 430\nout vec3 vNormal;\nvoid main() {\n    vNormal = vaNormal;\n    \
                   gl_Position = gl_ModelViewProjectionMatrix * vec4(vaPosition, 1.0);\n}\n";
        let given = |names: &[&str]| {
            names
                .iter()
                .map(|name| (name.to_string(), "hz_Vertex.xyz".to_owned()))
                .collect()
        };
        if let Err(errors) = fold_given(vsh, PASS_GSH, PASS_FSH, given(&["vaPosition", "vaNormal"]))
        {
            panic!("{errors:?}");
        }
        for kept in ["vaPosition", "vaNormal"] {
            assert!(
                fold_given(vsh, PASS_GSH, PASS_FSH, given(&[kept])).is_err(),
                "only {kept} given"
            );
        }
    }

    /// A struct of the vertex stage with members named as game inputs (`vaNormal`, `vaColor`)
    /// keeps them: the implicit inputs are renamed for the fold (`hz_vs_vaNormal`), the members
    /// and each access to them (`value.vaNormal`) aren't. Beside them the stage reads the real
    /// `vaNormal` and `vaPosition` undeclared. The stage's own names in a struct's body, a
    /// member's type (`Inner`) and an array's size (`N`), are renamed with their declarations.
    #[test]
    fn a_struct_member_named_as_a_game_input_keeps_its_name_under_a_geometry_stage() {
        let vsh = "#version 430\nconst int N = 2;\nstruct Inner { float a; };\n\
                   struct Sample { vec3 vaNormal; Inner[2] inner; float w, vaColor[N]; };\n\
                   out vec3 vNormal;\nvoid main() {\n    Sample value;\n    \
                   value.vaNormal = vaNormal;\n    value.vaColor[1] = 1.0;\n    \
                   value.inner[1].a = 2.0;\n    \
                   vNormal = value.vaNormal * value.vaColor[1] * value.inner[1].a;\n    \
                   gl_Position = gl_ModelViewProjectionMatrix * vec4(vaPosition, 1.0);\n}\n";
        let given = vec![
            ("vaPosition".to_owned(), "hz_Vertex.xyz".to_owned()),
            ("vaNormal".to_owned(), "hz_Vertex.xyz".to_owned()),
            ("vaColor".to_owned(), "hz_Color".to_owned()),
        ];
        if let Err(errors) = fold_given(vsh, PASS_GSH, PASS_FSH, given) {
            panic!("{errors:?}");
        }
    }

    /// The same with structs defined in a function, one in `main` and one in a block in it
    /// (GLSL 4.60, 4.1.8: a struct may be defined in any scope), under a vertex stage that
    /// never reads the game's `vaNormal` nor `vaColor`: their members keep their names too.
    #[test]
    fn a_struct_member_in_a_function_keeps_its_name_under_a_geometry_stage() {
        let vsh = "#version 430\nout vec3 vNormal;\nvoid main() {\n    \
                   struct Sample { vec3 vaNormal; float w, vaColor[2]; };\n    Sample value;\n    \
                   value.vaNormal = vec3(1.0);\n    value.vaColor[1] = 2.0;\n    \
                   vNormal = value.vaNormal * value.vaColor[1];\n    \
                   if (vNormal.x > 0.0) {\n        struct Inner { vec3 vaNormal; };\n        \
                   Inner inner;\n        inner.vaNormal = vNormal;\n        vNormal = inner.vaNormal;\n    }\n    \
                   gl_Position = gl_ModelViewProjectionMatrix * vec4(vaPosition, 1.0);\n}\n";
        let given = vec![
            ("vaPosition".to_owned(), "hz_Vertex.xyz".to_owned()),
            ("vaNormal".to_owned(), "hz_Vertex.xyz".to_owned()),
            ("vaColor".to_owned(), "hz_Color".to_owned()),
        ];
        if let Err(errors) = fold_given(vsh, PASS_GSH, PASS_FSH, given) {
            panic!("{errors:?}");
        }
    }

    /// The names of `module`'s globals, and of its structs' members.
    fn names(module: &naga::Module) -> (Vec<String>, Vec<String>) {
        let globals = (module.global_variables.iter())
            .filter_map(|(_, g)| g.name.clone())
            .collect();
        let members = (module.types.iter())
            .flat_map(|(_, t)| match &t.inner {
                naga::TypeInner::Struct { members, .. } => members.clone(),
                _ => Vec::new(),
            })
            .filter_map(|m| m.name)
            .collect();
        (globals, members)
    }

    /// The game's `vaPosition` and `vaNormal`, as the entities' inputs give them.
    fn position_and_normal() -> Vec<(String, String)> {
        vec![
            ("vaPosition".to_owned(), "hz_Vertex.xyz".to_owned()),
            ("vaNormal".to_owned(), "hz_Vertex.xyz".to_owned()),
        ]
    }

    /// A buffer block's field named `vaNormal` keeps its name under a geometry stage, as each
    /// access to it (`params.vaNormal`) does, beside the game's `vaNormal`, which the stage
    /// reads undeclared, given for the fold as `hz_vs_vaNormal`.
    #[test]
    fn a_buffer_field_named_as_a_game_input_keeps_its_name_under_a_geometry_stage() {
        let vsh = "#version 430\nlayout(std430, binding = 2) buffer Params { float w; vec3 vaNormal; } params;\n\
                   out vec3 vNormal;\nvoid main() {\n    vNormal = params.vaNormal + vaNormal;\n    \
                   gl_Position = gl_ModelViewProjectionMatrix * vec4(vaPosition, 1.0);\n}\n";
        let (program, _) = fold_given(vsh, PASS_GSH, PASS_FSH, position_and_normal())
            .unwrap_or_else(|e| panic!("{e:?}"));
        let (globals, members) = names(&program.vertex);
        assert!(members.iter().any(|m| m == "vaNormal"), "{members:?}");
        assert!(globals.iter().any(|g| g == "hz_vs_vaNormal"), "{globals:?}");
    }

    /// A block without an instance makes its members globals of the stage (GLSL 4.60, 4.3.9):
    /// a buffer block's field `vaNormal` is what the stage reads as `vaNormal`, and an `out`
    /// block's member `vaNormal` what it writes (an output of the fold's, `hz_vs_vaNormal`), so
    /// the game gives no `vaNormal` of its own beside them: it would be declared twice.
    #[test]
    fn a_block_member_without_an_instance_is_the_stages_own_under_a_geometry_stage() {
        let vsh = |block: &str, read: &str| {
            format!(
                "#version 430\n{block}\nout vec3 vNormal;\nvoid main() {{\n    {read}\n    \
                 gl_Position = gl_ModelViewProjectionMatrix * vec4(vaPosition, 1.0);\n}}\n"
            )
        };
        let buffer = vsh(
            "layout(std430, binding = 2) buffer Params { float w; vec3 vaNormal; };",
            "vNormal = vaNormal;",
        );
        let (program, _) = fold_given(&buffer, PASS_GSH, PASS_FSH, position_and_normal())
            .unwrap_or_else(|e| panic!("{e:?}"));
        let (globals, members) = names(&program.vertex);
        assert!(members.iter().any(|m| m == "vaNormal"), "{members:?}");
        assert!(
            !globals.iter().any(|g| g == "hz_vs_vaNormal"),
            "{globals:?}"
        );
        let output = vsh(
            "out Extra { vec3 vaNormal; };",
            "vaNormal = vec3(1.0);\n    vNormal = vaNormal;",
        );
        if let Err(errors) = fold_given(&output, PASS_GSH, PASS_FSH, position_and_normal()) {
            panic!("{errors:?}");
        }
    }

    /// Stages that pass their varyings in interface blocks (GLSL 4.60, 4.3.9), members with
    /// their own qualifiers: the vertex stage's `out` block, the geometry stage's `in` block
    /// array and `out` block, the fragment stage's `in` block. The geometry stage's two blocks
    /// share a name, as an input and an output block may (4.3.9).
    #[test]
    fn interface_blocks_pass_between_the_stages() {
        let vsh = "#version 430\nuniform float far;\n\
                   out VertexData {\n    vec4 color;\n    flat float id;\n    vec2 uv;\n} vOut;\n\
                   void main() {\n    vOut.color = gl_Color;\n    vOut.id = far;\n    \
                   vOut.uv = gl_Vertex.xy;\n    gl_Position = ftransform();\n}\n";
        let gsh = "#version 430\nlayout(triangles) in;\nlayout(triangle_strip, max_vertices = 3) out;\n\
                   in VertexData {\n    vec4 color;\n    flat float id;\n    vec2 uv;\n} vIn[];\n\
                   out VertexData {\n    vec4 color;\n    flat float id;\n} gOut;\n\
                   void main() {\n    for (int i = 0; i < 3; i++) {\n        \
                   gOut.color = vIn[i].color * vIn[i].uv.x;\n        gOut.id = vIn[0].id;\n        \
                   gl_Position = gl_in[i].gl_Position;\n        EmitVertex();\n    }\n    \
                   EndPrimitive();\n}\n";
        let fsh = "#version 430\nuniform float far;\n\
                   in VertexData {\n    vec4 color;\n    flat float id;\n} gIn;\n\
                   void main() {\n    gl_FragData[0] = gIn.color * gIn.id / far;\n}\n";
        let (program, triangles) = fold(vsh, gsh, fsh).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(triangles, 1);
        assert_eq!(program.fragment.entry_points.len(), 1);
    }

    /// An output qualified `sample` declares no variable named after its type (GLSL 4.60, 4.3).
    #[test]
    fn sample_outputs_rename_no_type() {
        let vsh = "#version 430\nuniform float far;\n\
                   out sample float vDepth;\nvoid main() {\n    vDepth = far;\n    \
                   gl_Position = ftransform();\n}\n";
        let gsh = "#version 430\nlayout(triangles) in;\nlayout(triangle_strip, max_vertices = 3) out;\n\
                   in float vDepth[];\nout float gDepth;\nvoid main() {\n    \
                   for (int i = 0; i < 3; i++) {\n        gDepth = vDepth[i];\n        \
                   gl_Position = gl_in[i].gl_Position;\n        EmitVertex();\n    }\n    \
                   EndPrimitive();\n}\n";
        let fsh = "#version 430\nuniform float far;\nin float gDepth;\n\
                   void main() {\n    gl_FragData[0] = vec4(gDepth / far);\n}\n";
        fold(vsh, gsh, fsh).unwrap_or_else(|e| panic!("{e:?}"));
    }

    /// An unflattened `out` or `in` interface block (such as one with a malformed empty member,
    /// a qualifier-and-layout prefix, or a body token named `buffer`) stays in place rather than
    /// being blanked or renamed, so its refusal points at the authored stage file and line.
    #[test]
    fn malformed_unflattened_interface_blocks_keep_their_authored_lines() {
        let bad_vsh = "#version 430\nout VertexData {\n    vec2 uv;\n    ;\n} vOut;\n\
                       void main() {\n    gl_Position = ftransform();\n}\n";
        let Err(errors) = fold(bad_vsh, PASS_GSH, PASS_FSH) else {
            panic!("a malformed vertex out block must be refused");
        };
        assert!(
            errors
                .iter()
                .any(|error| error.file == "shadow.vsh" && error.line == 4),
            "{errors:?}"
        );

        let qualified_vsh = "#version 430\nflat layout(location = 0) out VertexData {\n    \
                             vec2 uv;\n    ;\n} vOut;\n\
                             void main() {\n    gl_Position = ftransform();\n}\n";
        let Err(errors) = fold(qualified_vsh, PASS_GSH, PASS_FSH) else {
            panic!("a qualified malformed vertex out block must be refused");
        };
        assert!(
            errors
                .iter()
                .any(|error| error.file == "shadow.vsh" && error.line == 4),
            "{errors:?}"
        );

        let body_buffer_vsh = "#version 430\nout VertexData {\n    buffer Params;\n    ;\n} vOut;\n\
                               void main() {\n    gl_Position = ftransform();\n}\n";
        let buffer_gsh = format!(
            "#version 430\nlayout(std430, binding = 2) buffer Params {{ float w; }} params;\n{}",
            PASS_GSH.trim_start_matches("#version 430\n")
        );
        let Err(errors) = fold(body_buffer_vsh, &buffer_gsh, PASS_FSH) else {
            panic!("a vertex out block with a malformed buffer member must be refused");
        };
        assert!(
            errors
                .iter()
                .any(|error| error.file == "shadow.vsh" && error.line == 3),
            "{errors:?}"
        );

        let good_vsh = "#version 430\nout vec3 vNormal;\nvoid main() {\n    \
                        vNormal = vec3(0.0);\n    gl_Position = ftransform();\n}\n";
        let bad_gsh = "#version 430\nlayout(triangles) in;\n\
                       layout(triangle_strip, max_vertices = 3) out;\n\
                       in VertexData {\n    vec2 uv;\n    ;\n} vIn[];\n\
                       out vec3 gNormal;\nvoid main() {\n    \
                       for (int i = 0; i < 3; i++) {\n        gNormal = vec3(0.0);\n        \
                       gl_Position = gl_in[i].gl_Position;\n        EmitVertex();\n    }\n    \
                       EndPrimitive();\n}\n";
        let Err(errors) = fold(good_vsh, bad_gsh, PASS_FSH) else {
            panic!("a malformed geometry in block must be refused");
        };
        assert!(
            errors
                .iter()
                .any(|error| error.file == "shadow.gsh" && error.line == 6),
            "{errors:?}"
        );
    }

    #[test]
    fn multiline_atomic_arguments_keep_the_folded_error_origins() {
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                (
                    "shaders/shadow.gsh",
                    "layout(triangles) in;\nlayout(triangle_strip, max_vertices = 3) out;\n\
                     #include \"lib/emit.glsl\"\nvoid main() { emit(); }",
                ),
                (
                    "shaders/lib/emit.glsl",
                    "layout(std430, binding = 2) buffer Counters { int counter; };\n\
                     void emit() {\n    int old = atomicAdd(\n        counter, 1);\n    bad_geometry;\n}",
                ),
            ]),
        )
        .unwrap();
        let geometry = preprocess(&pack, "shadow.gsh", &Macros::standard());
        let vertex = Preprocessed::from_source("shadow.vsh", "void main() { bad_vertex; }");
        let (folded, _) = combine(&vertex, &geometry, &GeometryInputs::default()).unwrap();
        let tokens = lex(folded.code());
        for (name, expected) in [
            ("bad_geometry", ("lib/emit.glsl", 5)),
            ("bad_vertex", ("shadow.vsh", 1)),
        ] {
            let at = tokens.iter().position(|&token| token == name).unwrap();
            let error = crate::translate::scan::error_at_token(&folded, &tokens, at, name);
            assert_eq!((error.file.as_str(), error.line), expected);
        }
    }

    #[test]
    fn multiline_vertex_layout_keeps_the_next_error_origin() {
        let vertex = Preprocessed::from_source(
            "shadow.vsh",
            "layout(\n\n    location = 0) in vec3 position;\nvoid main() { bad_vertex; }",
        );
        let geometry = Preprocessed::from_source("shadow.gsh", PASS_GSH);
        let (folded, _) = combine(&vertex, &geometry, &GeometryInputs::default()).unwrap();
        let tokens = lex(folded.code());
        let at = tokens
            .iter()
            .position(|&token| token == "bad_vertex")
            .unwrap();
        let error = crate::translate::scan::error_at_token(&folded, &tokens, at, "bad_vertex");
        assert_eq!((error.file.as_str(), error.line), ("shadow.vsh", 4));
    }

    #[test]
    fn cancelling_stage_line_count_deltas_are_still_refused() {
        let vertex = Preprocessed::from_source("shadow.vsh", "float a;\nfloat b;");
        let geometry = Preprocessed::from_source("shadow.gsh", "float c;\nfloat d;");
        // Vertex loses one line and geometry gains one (total unchanged: 4 == 4).
        let err = join_stages(
            &vertex,
            "float a;",
            &geometry,
            "float c;\nfloat d;\nfloat extra;",
            "void head();",
            "void emit();",
            "void main() {}",
        )
        .unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!((err[0].file.as_str(), err[0].line), ("shadow.gsh", 0));
        // Geometry matches while vertex loses one line: refused at the vertex stage file.
        let err = join_stages(
            &vertex,
            "float a;",
            &geometry,
            "float c;\nfloat d;",
            "void head();",
            "void emit();",
            "void main() {}",
        )
        .unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!((err[0].file.as_str(), err[0].line), ("shadow.vsh", 0));
    }

    #[test]
    fn writes_run_once_and_keep_their_else() {
        let code =
            "if (c) imageStore(a, p, v); else imageStore(b, p, v);\nint n = atomicAdd(m, 1);\n";
        let tokens = lex(code);
        let mut text: Vec<String> = tokens.iter().map(|t| t.to_string()).collect();
        once(&mut text, &tokens);
        assert_eq!(
            text.concat(),
            "if (c) if (!hz_primary) {} else imageStore(a, p, v); else if (!hz_primary) {} else \
             imageStore(b, p, v);\nint n = (hz_primary ? atomicAdd(m, 1) : (m));\n"
        );
    }

    #[test]
    fn deeply_nested_geometry_stage_is_refused_with_location_before_combine() {
        let depth = sunrise_glsl::guard::MAX_NESTING;
        let vsh = "#version 430\nvoid main() {\n    gl_Position = ftransform();\n}\n";
        let gsh = format!(
            "#version 430\nlayout(triangles) in;\nlayout(triangle_strip, max_vertices = 3) out;\n\
             void main() {{\n    float x = {}1.0{};\n    for (int i = 0; i < 3; i++) {{\n        \
             gl_Position = gl_in[i].gl_Position * x;\n        EmitVertex();\n    }}\n    \
             EndPrimitive();\n}}\n",
            "(".repeat(depth),
            ")".repeat(depth),
        );
        let fsh = "#version 430\nvoid main() {\n    gl_FragData[0] = vec4(1.0);\n}\n";
        let errors = fold(vsh, &gsh, fsh)
            .err()
            .expect("deeply nested geometry stage must be refused");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "shadow.gsh");
        assert_eq!(errors[0].line, 5);
        assert_eq!(errors[0].stage, Some(naga::ShaderStage::Vertex));
        assert!(
            errors[0].message.contains("nesting exceeds"),
            "{}",
            errors[0].message
        );
    }

    #[test]
    fn geometry_stage_at_nesting_limit_translates_on_shader_pack_stack() {
        const SHADER_PACK_STACK_BYTES: usize = 8 * 1024 * 1024;
        let handle = std::thread::Builder::new()
            .stack_size(SHADER_PACK_STACK_BYTES)
            .spawn(|| {
                let depth = sunrise_glsl::guard::MAX_NESTING - 1;
                let vsh = "#version 430\nvoid main() {\n    gl_Position = ftransform();\n}\n";
                let gsh = format!(
                    "#version 430\nlayout(triangles) in;\nlayout(triangle_strip, max_vertices = 3) out;\n\
                     void main() {{\n    float x = {}1.0{};\n    for (int i = 0; i < 3; i++) {{\n        \
                     gl_Position = gl_in[i].gl_Position * x;\n        EmitVertex();\n    }}\n    \
                     EndPrimitive();\n}}\n",
                    "(".repeat(depth),
                    ")".repeat(depth),
                );
                let fsh = "#version 430\nvoid main() {\n    gl_FragData[0] = vec4(1.0);\n}\n";
                let result = fold(vsh, &gsh, fsh);
                assert!(result.is_ok(), "{:?}", result.err());
            })
            .unwrap();
        handle.join().unwrap();
    }
}
