//! Search over every visible page (guides spec 9.6; Q-049, Q-050): case-insensitive word-prefix matching of every
//! query word, titles ranked before text, then by the number of matches.

use super::library::Library;

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub page: usize,
    /// A short excerpt around the first match in the text, or "" for a title match only.
    pub excerpt: String,
    /// The anchor of the heading the first text match is under.
    pub anchor: Option<String>,
}

/// Letters that are written without spaces between words: each is a word of its own.
fn unspaced(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0xac00..=0xd7af | 0x0e00..=0x0e7f)
}

/// Lowercased words with their byte offsets in `text`.
fn words(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if unspaced(c) {
            if let Some(s) = start.take() {
                out.push((s, text[s..i].to_lowercase()));
            }
            out.push((i, c.to_string()));
        } else if c.is_alphanumeric() {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            out.push((s, text[s..i].to_lowercase()));
        }
    }
    if let Some(s) = start {
        out.push((s, text[s..].to_lowercase()));
    }
    out
}

/// Matches of every query word in `text`: the count, and the first match's offset; none when a word has no match.
fn matches(query: &[String], text: &str) -> Option<(usize, usize)> {
    let ws = words(text);
    let mut count = 0;
    let mut first = usize::MAX;
    for q in query {
        let mut any = false;
        for (at, w) in &ws {
            if w.starts_with(q.as_str()) {
                any = true;
                count += 1;
                first = first.min(*at);
            }
        }
        if !any {
            return None;
        }
    }
    Some((count, first))
}

fn excerpt(text: &str, at: usize) -> String {
    const BEFORE: usize = 30;
    const LEN: usize = 90;
    let start = text[..at]
        .char_indices()
        .rev()
        .nth(BEFORE)
        .map_or(0, |(i, _)| i);
    let shown: String = text[start..].chars().take(LEN).collect();
    let shown = shown.replace('\n', " ");
    let mut out = String::new();
    if start > 0 {
        out.push_str("...");
    }
    out.push_str(shown.trim());
    if text[start..].chars().count() > LEN {
        out.push_str("...");
    }
    out
}

pub fn search(lib: &Library, query: &str) -> Vec<Hit> {
    let query: Vec<String> = words(query).into_iter().map(|(_, w)| w).collect();
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (i, page) in lib.pages.iter().enumerate() {
        if page.hidden.is_some() {
            continue;
        }
        let title = matches(&query, &page.title);
        let mut text_hit = None;
        let mut text_count = 0;
        for (anchor, text) in page.doc.sections() {
            if let Some((count, at)) = matches(&query, &text) {
                text_count += count;
                if text_hit.is_none() {
                    text_hit = Some((anchor, excerpt(&text, at)));
                }
            }
        }
        if title.is_none() && text_hit.is_none() {
            // Every word may still match across sections.
            let all: String = page
                .doc
                .sections()
                .into_iter()
                .map(|(_, t)| t)
                .collect::<Vec<_>>()
                .join("\n");
            match matches(&query, &all) {
                Some((count, at)) => {
                    text_count = count;
                    text_hit = Some((None, excerpt(&all, at)));
                }
                None => continue,
            }
        }
        let (anchor, excerpt) = text_hit.unwrap_or((None, String::new()));
        let rank = (
            title.is_none(),
            usize::MAX - text_count - title.map_or(0, |t| t.0),
        );
        hits.push((
            rank,
            page.title.to_lowercase(),
            Hit {
                page: i,
                excerpt,
                anchor,
            },
        ));
    }
    hits.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    hits.into_iter().map(|(_, _, h)| h).collect()
}
