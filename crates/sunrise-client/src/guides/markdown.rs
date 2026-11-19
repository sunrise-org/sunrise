//! The Markdown of guide pages (guides spec section 4): CommonMark blocks and inlines with GFM tables and
//! strikethrough, and the MDX-like tags (`<Name attr="v">children</Name>`) guide pages use. Written from the
//! spec and CommonMark's rules; no other implementation was read.

use std::collections::HashMap;

/// An attribute value (spec 4.6): `"text"`, `{expression}` or a bare name.
#[derive(Clone, Debug, PartialEq)]
pub enum Attr {
    Str(String),
    Expr(String),
    Flag,
}

/// A tag's attributes in written order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attrs(pub Vec<(String, Attr)>);

impl Attrs {
    pub fn get(&self, name: &str) -> Option<&Attr> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
    /// The value as text: a string, or an expression's text (a quoted string literal in braces unquoted).
    pub fn str(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            Attr::Str(s) => Some(s),
            Attr::Expr(e) => {
                let e = e.trim();
                Some(
                    e.strip_prefix('"')
                        .and_then(|e| e.strip_suffix('"'))
                        .unwrap_or(e),
                )
            }
            Attr::Flag => None,
        }
    }
    /// A boolean: a bare name, `{true}` or `"true"` (Q-011).
    pub fn flag(&self, name: &str) -> Option<bool> {
        match self.get(name)? {
            Attr::Flag => Some(true),
            _ => match self.str(name)?.trim() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
        }
    }
    /// A number written `"2.5"` or `{2.5}`.
    pub fn num(&self, name: &str) -> Option<f32> {
        self.str(name)?
            .trim()
            .parse()
            .ok()
            .filter(|v: &f32| v.is_finite())
    }
    /// The `p:<name>` block state properties.
    pub fn props(&self) -> Vec<(String, String)> {
        self.0
            .iter()
            .filter_map(|(n, _)| {
                let prop = n.strip_prefix("p:")?;
                Some((prop.to_owned(), self.str(n)?.to_owned()))
            })
            .collect()
    }
}

/// A tag and its children: blocks for a tag on its own lines, inlines inside a paragraph.
#[derive(Clone, Debug, PartialEq)]
pub struct Tag<C> {
    pub name: String,
    pub attrs: Attrs,
    pub children: Vec<C>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String),
    Code(String),
    Strong(Vec<Inline>),
    Emph(Vec<Inline>),
    Strike(Vec<Inline>),
    Link {
        target: String,
        children: Vec<Inline>,
    },
    Image {
        src: String,
        alt: String,
    },
    /// A hard line break.
    Break,
    Tag(Tag<Inline>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading {
        level: u8,
        text: Vec<Inline>,
        anchor: String,
    },
    Paragraph(Vec<Inline>),
    Quote(Vec<Block>),
    List {
        ordered: bool,
        start: u32,
        items: Vec<Vec<Block>>,
    },
    Code(String),
    Rule,
    Table {
        align: Vec<Align>,
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Tag(Tag<Block>),
    /// A tag that is never closed (its name), shown as an error (spec 4.13).
    Unclosed(String),
}

/// A parsed page body.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub blocks: Vec<Block>,
}

/// Lowercase element names read as tags (spec 4.11); any other lowercase `<name>` is text (Q-014).
const HTML: [&str; 3] = ["a", "br", "div"];
/// Tags that sit in a paragraph's text even alone on a line.
const INLINE: [&str; 7] = [
    "ItemLink",
    "Color",
    "KeyBind",
    "PlayerName",
    "CommandLink",
    "a",
    "br",
];

pub fn parse(body: &str) -> Document {
    let text = strip_comments(&body.replace("\r\n", "\n").replace('\t', "    "));
    let mut blocks = parse_blocks(&text);
    let mut seen = HashMap::new();
    assign_anchors(&mut blocks, &mut seen);
    Document { blocks }
}

/// Removes `{/* ... */}` comments (spec 4.9), outside fenced code; the lines they covered stay, empty.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<String> = None;
    let mut rest = text;
    while !rest.is_empty() {
        let line_end = rest.find('\n').map_or(rest.len(), |i| i + 1);
        let line = &rest[..line_end];
        let trimmed = line.trim_start();
        if let Some(f) = &fence {
            if trimmed.starts_with(f.as_str()) {
                fence = None;
            }
            out.push_str(line);
            rest = &rest[line_end..];
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(trimmed[..3].to_owned());
            out.push_str(line);
            rest = &rest[line_end..];
            continue;
        }
        match line.find("{/*") {
            Some(start) => {
                out.push_str(&line[..start]);
                let after = &rest[start + 3..];
                match after.find("*/}") {
                    Some(end) => {
                        out.extend(after[..end].chars().filter(|&c| c == '\n'));
                        rest = &after[end + 3..];
                    }
                    None => rest = "",
                }
            }
            None => {
                out.push_str(line);
                rest = &rest[line_end..];
            }
        }
    }
    out
}

/// Heading anchors (spec 5.2.3, Q-016): the heading's plain text lowercased, letters, digits, `-` and `_`
/// kept, spaces as `-`, other characters dropped; a repeated anchor gets `-1`, `-2`... in page order.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            out.extend(c.to_lowercase());
        } else if c.is_whitespace() {
            out.push('-');
        }
    }
    out
}

fn assign_anchors(blocks: &mut [Block], seen: &mut HashMap<String, usize>) {
    for b in blocks {
        match b {
            Block::Heading { text, anchor, .. } => {
                let base = slug(&plain(text));
                let n = seen.entry(base.clone()).or_insert(0);
                *anchor = if *n == 0 { base } else { format!("{base}-{n}") };
                *n += 1;
            }
            Block::Quote(inner) => assign_anchors(inner, seen),
            Block::List { items, .. } => {
                for item in items {
                    assign_anchors(item, seen);
                }
            }
            Block::Tag(t) => assign_anchors(&mut t.children, seen),
            _ => {}
        }
    }
}

/// Inlines as plain text: what search and anchors read.
pub fn plain(inlines: &[Inline]) -> String {
    let mut out = String::new();
    plain_into(inlines, &mut out);
    out
}

fn plain_into(inlines: &[Inline], out: &mut String) {
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) => out.push_str(t),
            Inline::Strong(c) | Inline::Emph(c) | Inline::Strike(c) => plain_into(c, out),
            Inline::Link { children, .. } => plain_into(children, out),
            Inline::Tag(t) => plain_into(&t.children, out),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::Break => out.push(' '),
        }
    }
}

impl Document {
    /// The first level-1 heading's text (spec 3.5).
    pub fn title(&self) -> Option<String> {
        fn find(blocks: &[Block]) -> Option<String> {
            blocks.iter().find_map(|b| match b {
                Block::Heading { level: 1, text, .. } => Some(plain(text)),
                Block::Tag(t) => find(&t.children),
                _ => None,
            })
        }
        find(&self.blocks)
    }
    /// The page's text for search: each block's text on its own line, with the anchor of the heading it is
    /// under.
    pub fn sections(&self) -> Vec<(Option<String>, String)> {
        fn walk(blocks: &[Block], out: &mut Vec<(Option<String>, String)>) {
            for b in blocks {
                match b {
                    Block::Heading { text, anchor, .. } => {
                        out.push((Some(anchor.clone()), String::new()));
                        push(out, &plain(text));
                    }
                    Block::Paragraph(i) => push(out, &plain(i)),
                    Block::Quote(c) => walk(c, out),
                    Block::List { items, .. } => {
                        for item in items {
                            walk(item, out);
                        }
                    }
                    Block::Code(c) => push(out, c),
                    Block::Table { head, rows, .. } => {
                        for cell in head.iter().chain(rows.iter().flatten()) {
                            push(out, &plain(cell));
                        }
                    }
                    Block::Tag(t) => walk(&t.children, out),
                    Block::Rule | Block::Unclosed(_) => {}
                }
            }
        }
        fn push(out: &mut Vec<(Option<String>, String)>, text: &str) {
            if out.is_empty() {
                out.push((None, String::new()));
            }
            let last = &mut out.last_mut().expect("pushed").1;
            if !last.is_empty() {
                last.push('\n');
            }
            last.push_str(text);
        }
        let mut out = Vec::new();
        walk(&self.blocks, &mut out);
        out
    }
}

// ---- Tags ----

/// An opening tag read at the start of a string.
#[derive(Debug)]
struct Open {
    name: String,
    attrs: Attrs,
    self_closing: bool,
    len: usize,
}

fn is_tag_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase()) || HTML.contains(&name)
}

fn name_len(s: &str) -> usize {
    s.bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
        .count()
}

/// `<Name a="v" b={1} c />` at the start of `s`.
fn open_tag(s: &str) -> Option<Open> {
    let b = s.as_bytes();
    if b.first() != Some(&b'<') {
        return None;
    }
    let n = name_len(&s[1..]);
    let name = &s[1..1 + n];
    if n == 0 || !name.starts_with(|c: char| c.is_ascii_alphabetic()) || !is_tag_name(name) {
        return None;
    }
    let mut i = 1 + n;
    let mut attrs = Vec::new();
    loop {
        let ws = s[i..].len() - s[i..].trim_start().len();
        i += ws;
        match b.get(i)? {
            b'>' => {
                return Some(Open {
                    name: name.to_owned(),
                    attrs: Attrs(attrs),
                    self_closing: false,
                    len: i + 1,
                });
            }
            b'/' if b.get(i + 1) == Some(&b'>') => {
                return Some(Open {
                    name: name.to_owned(),
                    attrs: Attrs(attrs),
                    self_closing: true,
                    len: i + 2,
                });
            }
            _ => {}
        }
        if ws == 0 {
            return None;
        }
        let an = name_len(&s[i..]);
        if an == 0 {
            return None;
        }
        let attr = s[i..i + an].to_owned();
        i += an;
        let after = s[i..].trim_start();
        if !after.starts_with('=') {
            attrs.push((attr, Attr::Flag));
            continue;
        }
        i = s.len() - after.len() + 1;
        i += s[i..].len() - s[i..].trim_start().len();
        match b.get(i)? {
            q @ (b'"' | b'\'') => {
                let end = s[i + 1..].find(*q as char)?;
                attrs.push((attr, Attr::Str(decode_entities(&s[i + 1..i + 1 + end]))));
                i += end + 2;
            }
            b'{' => {
                let mut depth = 0usize;
                let mut quote: Option<u8> = None;
                let mut end = None;
                for (k, &c) in b[i..].iter().enumerate() {
                    match (quote, c) {
                        (Some(q), c) if c == q => quote = None,
                        (Some(_), _) => {}
                        (None, b'"' | b'\'' | b'`') => quote = Some(c),
                        (None, b'{') => depth += 1,
                        (None, b'}') => {
                            depth -= 1;
                            if depth == 0 {
                                end = Some(i + k);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let end = end?;
                attrs.push((attr, Attr::Expr(s[i + 1..end].to_owned())));
                i = end + 1;
            }
            _ => return None,
        }
    }
}

/// `</Name>` at the start of `s`: the name and the length.
fn close_tag(s: &str) -> Option<(&str, usize)> {
    let rest = s.strip_prefix("</")?;
    let n = name_len(rest);
    let after = &rest[n..];
    let trimmed = after.trim_start();
    trimmed
        .starts_with('>')
        .then(|| (&rest[..n], 2 + n + (after.len() - trimmed.len()) + 1))
}

/// Where the tag `name` opened just before `s` closes: the children's end and the end of the closing tag.
fn find_close(s: &str, name: &str) -> Option<(usize, usize)> {
    let mut depth = 1usize;
    let mut i = 0;
    let b = s.as_bytes();
    while i < b.len() {
        match b[i] {
            b'`' => {
                // Code spans hide tags.
                let run = b[i..].iter().take_while(|&&c| c == b'`').count();
                let fence = &s[i..i + run];
                match s[i + run..].find(fence) {
                    Some(end) => i += run + end + run,
                    None => i += run,
                }
                continue;
            }
            b'<' => {
                if let Some((n, len)) = close_tag(&s[i..]) {
                    if n == name {
                        depth -= 1;
                        if depth == 0 {
                            return Some((i, i + len));
                        }
                    }
                    i += len;
                    continue;
                }
                if let Some(open) = open_tag(&s[i..]) {
                    if open.name == name && !open.self_closing {
                        depth += 1;
                    }
                    i += open.len;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Removes the indentation every non-blank line shares.
fn dedent(text: &str) -> String {
    let indent = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    text.lines()
        .map(|l| {
            if l.len() >= indent {
                &l[indent..]
            } else {
                l.trim_start()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---- Blocks ----

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let t = line.trim_start();
    if indent_of(line) > 3 {
        return None;
    }
    let level = t.bytes().take_while(|&c| c == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &t[level..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let mut text = rest.trim();
    // A closing run of #s preceded by a space.
    let stripped = text.trim_end_matches('#');
    if stripped.is_empty() || stripped.ends_with(' ') {
        text = stripped.trim_end();
    }
    Some((level as u8, text))
}

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    if indent_of(line) > 3 || t.len() < 3 {
        return false;
    }
    let c = t.as_bytes()[0];
    matches!(c, b'-' | b'*' | b'_')
        && t.bytes().filter(|&b| b != b' ').all(|b| b == c)
        && t.bytes().filter(|&b| b == c).count() >= 3
}

fn fence(line: &str) -> Option<(char, usize)> {
    let t = line.trim_start();
    let c = t.chars().next()?;
    if indent_of(line) > 3 || !matches!(c, '`' | '~') {
        return None;
    }
    let n = t.chars().take_while(|&x| x == c).count();
    (n >= 3 && !(c == '`' && t[n..].contains('`'))).then_some((c, n))
}

/// A list item's marker: ordered, its number, and the content's column.
fn list_marker(line: &str) -> Option<(bool, u32, usize)> {
    let indent = indent_of(line);
    let t = &line[indent..];
    let (ordered, number, mlen) = match t.as_bytes().first()? {
        b'-' | b'*' | b'+' => (false, 1, 1),
        b'0'..=b'9' => {
            let digits = t.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 9 || !matches!(t.as_bytes().get(digits), Some(b'.' | b')')) {
                return None;
            }
            (true, t[..digits].parse().ok()?, digits + 1)
        }
        _ => return None,
    };
    let after = &t[mlen..];
    if after.is_empty() {
        return Some((ordered, number, indent + mlen + 1));
    }
    if !after.starts_with(' ') {
        return None;
    }
    let spaces = indent_of(after).min(4);
    Some((ordered, number, indent + mlen + spaces.max(1)))
}

fn is_delimiter_row(line: &str) -> bool {
    let t = line.trim();
    if !t.contains('-') || !t.contains(['|', '-']) {
        return false;
    }
    let cells = split_row(t);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c = c.trim();
            let core = c.trim_start_matches(':').trim_end_matches(':');
            !core.is_empty() && core.bytes().all(|b| b == b'-')
        })
        && (t.contains('|') || cells.len() > 1)
}

/// A table row's cells: split at `|` outside code spans and not escaped, without the outer pipes.
fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with('|') && !t.ends_with("\\|") {
        &t[..t.len() - 1]
    } else {
        t
    };
    let mut cells = vec![String::new()];
    let mut code = false;
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cells.last_mut().expect("one cell").push('|');
                chars.next();
            }
            '`' => {
                code = !code;
                cells.last_mut().expect("one cell").push(c);
            }
            '|' if !code => cells.push(String::new()),
            _ => cells.last_mut().expect("one cell").push(c),
        }
    }
    cells
}

/// A tag line that makes a block: the tag, and the byte length of the source it took (through its line's end).
fn block_tag(text: &str) -> Option<(Block, usize)> {
    let lead = indent_of(text);
    let s = &text[lead..];
    let open = open_tag(s)?;
    let line_end = |from: usize| s[from..].find('\n').map_or(s.len(), |i| from + i + 1);
    let rest_blank = |from: usize| s[from..line_end(from)].trim().is_empty();
    if INLINE.contains(&open.name.as_str())
        && !(open.name == "br" && open.attrs.get("clear").is_some())
    {
        return None;
    }
    let tag = |children| {
        Block::Tag(Tag {
            name: open.name.clone(),
            attrs: open.attrs.clone(),
            children,
        })
    };
    if open.self_closing || (open.name == "br" && !s[open.len..].contains("</br")) {
        return rest_blank(open.len).then(|| (tag(Vec::new()), lead + line_end(open.len)));
    }
    match find_close(&s[open.len..], &open.name) {
        Some((inner_end, after)) => {
            let after = open.len + after;
            if !rest_blank(after) {
                return None;
            }
            let inner = &s[open.len..open.len + inner_end];
            let children = parse_blocks(&dedent(inner.trim_matches('\n')));
            Some((tag(children), lead + line_end(after)))
        }
        None if rest_blank(open.len) => Some((
            Block::Unclosed(open.name.clone()),
            lead + line_end(open.len),
        )),
        None => None,
    }
}

/// Whether a line begins a block that ends a paragraph.
fn interrupts(text_from_line: &str) -> bool {
    let line = text_from_line.lines().next().unwrap_or("");
    if line.trim().is_empty() {
        return true;
    }
    heading(line).is_some()
        || is_rule(line)
        || fence(line).is_some()
        || line.trim_start().starts_with('>')
        || list_marker(line).is_some_and(|(ordered, n, _)| {
            (!ordered || n == 1) && !line.trim_end().ends_with(['-', '*', '+'])
        })
        || block_tag(text_from_line).is_some()
}

fn parse_blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut pos = 0;
    let next_line = |pos: usize| text[pos..].find('\n').map_or(text.len(), |i| pos + i + 1);
    while pos < text.len() {
        let end = next_line(pos);
        let line = text[pos..end].trim_end_matches('\n');
        let rest = &text[pos..];
        if line.trim().is_empty() {
            pos = end;
            continue;
        }
        if indent_of(line) >= 4 {
            // Indented code; a paragraph can't be interrupted by it (it never reaches here).
            let mut code = Vec::new();
            let mut p = pos;
            while p < text.len() {
                let e = next_line(p);
                let l = text[p..e].trim_end_matches('\n');
                if !l.trim().is_empty() && indent_of(l) < 4 {
                    break;
                }
                code.push(l.get(4..).unwrap_or(""));
                p = e;
            }
            while code.last().is_some_and(|l| l.trim().is_empty()) {
                code.pop();
            }
            blocks.push(Block::Code(code.join("\n")));
            pos = p;
            continue;
        }
        if let Some((c, n)) = fence(line) {
            let indent = indent_of(line);
            let mut code = Vec::new();
            let mut p = end;
            while p < text.len() {
                let e = next_line(p);
                let l = text[p..e].trim_end_matches('\n');
                p = e;
                let t = l.trim();
                if t.chars().take_while(|&x| x == c).count() >= n && t.chars().all(|x| x == c) {
                    break;
                }
                let strip = indent_of(l).min(indent);
                code.push(&l[strip..]);
            }
            blocks.push(Block::Code(code.join("\n")));
            pos = p;
            continue;
        }
        if let Some((level, h)) = heading(line) {
            blocks.push(Block::Heading {
                level,
                text: parse_inlines(h),
                anchor: String::new(),
            });
            pos = end;
            continue;
        }
        if is_rule(line) {
            blocks.push(Block::Rule);
            pos = end;
            continue;
        }
        if line.trim_start().starts_with('>') {
            let mut inner = Vec::new();
            let mut p = pos;
            let mut lazy = false;
            while p < text.len() {
                let e = next_line(p);
                let l = text[p..e].trim_end_matches('\n');
                let t = l.trim_start();
                if let Some(q) = t.strip_prefix('>') {
                    let q = q.strip_prefix(' ').unwrap_or(q);
                    lazy = !q.trim().is_empty();
                    inner.push(q.to_owned());
                } else if lazy && !interrupts(&text[p..]) {
                    inner.push(l.to_owned());
                } else {
                    break;
                }
                p = e;
            }
            blocks.push(Block::Quote(parse_blocks(&inner.join("\n"))));
            pos = p;
            continue;
        }
        if let Some((ordered, start, _)) = list_marker(line) {
            let (items, p) = parse_list(text, pos, ordered);
            blocks.push(Block::List {
                ordered,
                start,
                items,
            });
            pos = p;
            continue;
        }
        if let Some((block, len)) = block_tag(rest) {
            blocks.push(block);
            pos += len;
            continue;
        }
        if line.contains('|') && end < text.len() {
            let next_end = next_line(end);
            let next = text[end..next_end].trim_end_matches('\n');
            if is_delimiter_row(next) {
                let head = split_row(line);
                let align = split_row(next)
                    .iter()
                    .map(|c| {
                        let c = c.trim();
                        match (c.starts_with(':'), c.ends_with(':')) {
                            (true, true) => Align::Center,
                            (true, false) => Align::Left,
                            (false, true) => Align::Right,
                            (false, false) => Align::None,
                        }
                    })
                    .collect::<Vec<_>>();
                let mut rows = Vec::new();
                let mut p = next_end;
                while p < text.len() {
                    let e = next_line(p);
                    let l = text[p..e].trim_end_matches('\n');
                    if l.trim().is_empty() || interrupts(&text[p..]) {
                        break;
                    }
                    let mut cells: Vec<_> = split_row(l)
                        .iter()
                        .map(|c| parse_inlines(c.trim()))
                        .collect();
                    cells.resize(head.len(), Vec::new());
                    rows.push(cells);
                    p = e;
                }
                blocks.push(Block::Table {
                    align,
                    head: head.iter().map(|c| parse_inlines(c.trim())).collect(),
                    rows,
                });
                pos = p;
                continue;
            }
        }
        // A paragraph, or a setext heading.
        let mut lines = vec![line.trim_start()];
        let mut p = end;
        let mut setext = None;
        while p < text.len() {
            let e = next_line(p);
            let l = text[p..e].trim_end_matches('\n');
            let t = l.trim();
            if indent_of(l) <= 3
                && !t.is_empty()
                && (t.bytes().all(|b| b == b'=') || t.bytes().all(|b| b == b'-'))
            {
                setext = Some(if t.starts_with('=') { 1 } else { 2 });
                p = e;
                break;
            }
            if interrupts(&text[p..]) {
                break;
            }
            lines.push(l.trim_start());
            p = e;
        }
        let joined = lines.join("\n");
        blocks.push(match setext {
            Some(level) => Block::Heading {
                level,
                text: parse_inlines(joined.trim()),
                anchor: String::new(),
            },
            None => Block::Paragraph(parse_inlines(&joined)),
        });
        pos = p;
    }
    blocks
}

/// A list starting at `pos`: its items' blocks and where it ends.
fn parse_list(text: &str, mut pos: usize, ordered: bool) -> (Vec<Vec<Block>>, usize) {
    let next_line = |pos: usize| text[pos..].find('\n').map_or(text.len(), |i| pos + i + 1);
    let mut items = Vec::new();
    while pos < text.len() {
        let end = next_line(pos);
        let line = text[pos..end].trim_end_matches('\n');
        let Some((o, _, content)) = list_marker(line) else {
            break;
        };
        if o != ordered || is_rule(line) {
            break;
        }
        let mut lines = vec![line.get(content..).unwrap_or("").to_owned()];
        let mut p = end;
        let mut blank = false;
        while p < text.len() {
            let e = next_line(p);
            let l = text[p..e].trim_end_matches('\n');
            if l.trim().is_empty() {
                blank = true;
                lines.push(String::new());
                p = e;
                continue;
            }
            if indent_of(l) >= content {
                lines.push(l[content..].to_owned());
            } else if !blank && !interrupts(&text[p..]) && list_marker(l).is_none() {
                lines.push(l.trim_start().to_owned());
            } else {
                break;
            }
            blank = false;
            p = e;
        }
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        items.push(parse_blocks(&lines.join("\n")));
        pos = p;
        // Blank lines between items are skipped; anything else ends the list.
        let mut q = pos;
        while q < text.len() && text[q..next_line(q)].trim().is_empty() {
            q = next_line(q);
        }
        if q < text.len()
            && list_marker(text[q..next_line(q)].trim_end_matches('\n'))
                .is_some_and(|m| m.0 == ordered)
        {
            pos = q;
        } else {
            break;
        }
    }
    (items, pos)
}

// ---- Inlines ----

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = tail.find(';').filter(|&e| e <= 10);
        let named = end.and_then(|e| {
            let name = &tail[1..e];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => '\u{a0}',
                _ => {
                    let n = name.strip_prefix('#')?;
                    let code = match n.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => n.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, e))
        });
        match named {
            Some((c, e)) => {
                out.push(c);
                rest = &tail[e + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn push_text(out: &mut Vec<Inline>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(Inline::Text(t)) = out.last_mut() {
        t.push_str(text);
    } else {
        out.push(Inline::Text(text.to_owned()));
    }
}

/// Where a code span opened at `i` (a run of backticks) closes: the content and the end.
fn code_span(s: &str, i: usize) -> Option<(&str, usize)> {
    let run = s[i..].bytes().take_while(|&b| b == b'`').count();
    let fence = &s[i..i + run];
    let mut from = i + run;
    loop {
        let at = from + s[from..].find(fence)?;
        let after = s[at..].bytes().take_while(|&b| b == b'`').count();
        if after == run {
            return Some((&s[i + run..at], at + run));
        }
        from = at + after;
    }
}

/// The `]` matching the `[` at `i`.
fn close_bracket(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0;
    let mut k = i;
    while k < b.len() {
        match b[k] {
            b'\\' => k += 1,
            b'`' => {
                if let Some((_, end)) = code_span(s, k) {
                    k = end;
                    continue;
                }
            }
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
        k += 1;
    }
    None
}

/// `(target "title")` at `i`: the target and the end.
fn link_target(s: &str, i: usize) -> Option<(String, usize)> {
    let b = s.as_bytes();
    if b.get(i) != Some(&b'(') {
        return None;
    }
    let mut depth = 0;
    let mut k = i;
    while k < b.len() {
        match b[k] {
            b'\\' => k += 1,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            b'\n' if depth > 0 && s[i + 1..k].trim().is_empty() => {}
            _ => {}
        }
        k += 1;
    }
    if k >= b.len() {
        return None;
    }
    let inner = s[i + 1..k].trim();
    let target = if let Some(t) = inner.strip_prefix('<') {
        t.split('>').next().unwrap_or("")
    } else {
        // A title after the target is dropped.
        inner.split_whitespace().next().unwrap_or("")
    };
    Some((target.to_owned(), k + 1))
}

/// Whether a delimiter run of `c` at `i..j` may open / close emphasis.
fn flanking(s: &str, i: usize, j: usize, c: u8) -> (bool, bool) {
    let before = s[..i].chars().next_back();
    let after = s[j..].chars().next();
    let ws = |c: Option<char>| c.is_none_or(char::is_whitespace);
    let alnum = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let open = !ws(after);
    let close = !ws(before);
    if c == b'_' {
        (open && !alnum(before), close && !alnum(after))
    } else {
        (open, close)
    }
}

/// The closing run of exactly `n` `c`s for an opener ending at `from`, skipping code spans and links.
fn find_closer(s: &str, from: usize, c: u8, n: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut k = from;
    while k < b.len() {
        match b[k] {
            b'\\' => k += 2,
            b'`' => match code_span(s, k) {
                Some((_, end)) => k = end,
                None => k += 1,
            },
            x if x == c => {
                let run = b[k..].iter().take_while(|&&y| y == c).count();
                let (_, close) = flanking(s, k, k + run, c);
                if close && k > from && (run == n || (n == 1 && run == 3) || (run > n && n == 2)) {
                    return Some(k);
                }
                k += run;
            }
            _ => k += 1,
        }
    }
    None
}

pub fn parse_inlines(s: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut text_start = 0;
    let mut i = 0;
    let flush = |out: &mut Vec<Inline>, from: usize, to: usize| {
        push_text(out, &decode_entities(&s[from..to]))
    };
    while i < b.len() {
        match b[i] {
            b'\\' if i + 1 < b.len() => {
                flush(&mut out, text_start, i);
                if b[i + 1] == b'\n' {
                    out.push(Inline::Break);
                    i += 2;
                } else if b[i + 1].is_ascii_punctuation() {
                    push_text(&mut out, &s[i + 1..i + 2]);
                    i += 2;
                } else {
                    push_text(&mut out, "\\");
                    i += 1;
                }
                text_start = i;
            }
            b'\n' => {
                // Trailing spaces before a newline: two or more make a hard break.
                let line = &s[text_start..i];
                let trimmed = line.trim_end_matches(' ');
                let hard = line.len() - trimmed.len() >= 2;
                flush(&mut out, text_start, text_start + trimmed.len());
                out.push(if hard {
                    Inline::Break
                } else {
                    Inline::Text(" ".into())
                });
                merge_text(&mut out);
                i += 1;
                while i < b.len() && b[i] == b' ' {
                    i += 1;
                }
                text_start = i;
            }
            b'`' => {
                if let Some((code, end)) = code_span(s, i) {
                    flush(&mut out, text_start, i);
                    let code = code.replace('\n', " ");
                    let trimmed = if code.len() > 2 && code.starts_with(' ') && code.ends_with(' ')
                    {
                        code[1..code.len() - 1].to_owned()
                    } else {
                        code
                    };
                    out.push(Inline::Code(trimmed));
                    i = end;
                    text_start = i;
                } else {
                    i += b[i..].iter().take_while(|&&c| c == b'`').count();
                }
            }
            b'!' if b.get(i + 1) == Some(&b'[') => {
                if let Some(close) = close_bracket(s, i + 1)
                    && let Some((src, end)) = link_target(s, close + 1)
                {
                    flush(&mut out, text_start, i);
                    out.push(Inline::Image {
                        src,
                        alt: plain(&parse_inlines(&s[i + 2..close])),
                    });
                    i = end;
                    text_start = i;
                } else {
                    i += 1;
                }
            }
            b'[' => {
                if let Some(close) = close_bracket(s, i)
                    && let Some((target, end)) = link_target(s, close + 1)
                {
                    flush(&mut out, text_start, i);
                    out.push(Inline::Link {
                        target,
                        children: parse_inlines(&s[i + 1..close]),
                    });
                    i = end;
                    text_start = i;
                } else {
                    i += 1;
                }
            }
            b'<' => {
                // An autolink, a tag, or plain text (Q-014).
                let rest = &s[i + 1..];
                let url = rest.find('>').map(|end| &rest[..end]);
                if let Some(url) = url
                    && (url.starts_with("http://") || url.starts_with("https://"))
                    && !url.contains(char::is_whitespace)
                {
                    let end = url.len();
                    flush(&mut out, text_start, i);
                    out.push(Inline::Link {
                        target: url.to_owned(),
                        children: vec![Inline::Text(url.to_owned())],
                    });
                    i += end + 2;
                    text_start = i;
                    continue;
                }
                if let Some(open) = open_tag(&s[i..]) {
                    let void = open.name == "br";
                    if open.self_closing || void {
                        flush(&mut out, text_start, i);
                        out.push(Inline::Tag(Tag {
                            name: open.name,
                            attrs: open.attrs,
                            children: Vec::new(),
                        }));
                        i += open.len;
                        text_start = i;
                        continue;
                    }
                    if let Some((inner_end, after)) = find_close(&s[i + open.len..], &open.name) {
                        flush(&mut out, text_start, i);
                        let inner = &s[i + open.len..i + open.len + inner_end];
                        out.push(Inline::Tag(Tag {
                            name: open.name,
                            attrs: open.attrs,
                            children: parse_inlines(inner.trim()),
                        }));
                        i += open.len + after;
                        text_start = i;
                        continue;
                    }
                    if let Some(close) = close_tag(&s[i..]) {
                        // A stray closing tag shows nothing.
                        flush(&mut out, text_start, i);
                        i += close.1;
                        text_start = i;
                        continue;
                    }
                }
                if let Some((name, len)) = close_tag(&s[i..])
                    && is_tag_name(name)
                {
                    flush(&mut out, text_start, i);
                    i += len;
                    text_start = i;
                    continue;
                }
                i += 1;
            }
            c @ (b'*' | b'_' | b'~') => {
                let run = b[i..].iter().take_while(|&&x| x == c).count();
                let (open, _) = flanking(s, i, i + run, c);
                let mut done = false;
                if open {
                    for n in [run.min(2), 1] {
                        if n == 0 || (c == b'~' && run > 2) {
                            continue;
                        }
                        if let Some(close) = find_closer(s, i + n, c, n) {
                            flush(&mut out, text_start, i);
                            let inner = parse_inlines(&s[i + n..close]);
                            out.push(match (c, n) {
                                (b'~', _) => Inline::Strike(inner),
                                (_, 2) => Inline::Strong(inner),
                                _ => Inline::Emph(inner),
                            });
                            i = close + n;
                            text_start = i;
                            done = true;
                            break;
                        }
                    }
                }
                if !done {
                    i += run;
                }
            }
            _ => i += 1,
        }
    }
    flush(&mut out, text_start, s.len());
    if let Some(Inline::Text(t)) = out.last_mut() {
        let trimmed = t.trim_end().len();
        t.truncate(trimmed);
        if t.is_empty() {
            out.pop();
        }
    }
    out
}

fn merge_text(out: &mut Vec<Inline>) {
    if out.len() >= 2
        && let [.., Inline::Text(a), Inline::Text(b)] = out.as_mut_slice()
    {
        let b = std::mem::take(b);
        a.push_str(&b);
        out.pop();
    }
}
