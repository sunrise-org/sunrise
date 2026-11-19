//! A guide page's front matter (guides spec section 3): the YAML block between `---` lines at the top.

use yaml_rust2::{Yaml, YamlLoader};

/// What the front matter says. Unknown keys are ignored (spec 3.4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrontMatter {
    /// Whether the page has a `navigation` map: then it is in the contents tree.
    pub navigation: bool,
    pub title: Option<String>,
    /// The icon's item id as written (the namespace defaults later, to the page's).
    pub icon: Option<String>,
    /// The icon's components in the `/give` component syntax (`id=value,...`), or "".
    pub icon_components: String,
    pub parent: Option<String>,
    pub position: Option<f64>,
    pub max_depth: Option<u32>,
    pub item_ids: Vec<String>,
    pub categories: Vec<String>,
    /// `requires_mods` (spec 7.3, Q-040): mod ids that must be installed, `!id` that must not be.
    pub requires_mods: Vec<String>,
}

/// Splits a page into its front matter's YAML (none without a block) and its body.
pub fn split(text: &str) -> (Option<&str>, &str) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let first_end = text.find('\n').map_or(text.len(), |i| i + 1);
    if text[..first_end].trim_end() != "---" {
        return (None, text);
    }
    let mut pos = first_end;
    while pos < text.len() {
        let end = text[pos..].find('\n').map_or(text.len(), |i| pos + i + 1);
        let line = text[pos..end].trim_end();
        if line == "---" || line == "..." {
            return (Some(&text[first_end..pos]), &text[end..]);
        }
        pos = end;
    }
    // No closing line: not front matter.
    (None, text)
}

fn string(y: &Yaml) -> Option<String> {
    match y {
        Yaml::String(s) | Yaml::Real(s) => Some(s.clone()),
        Yaml::Integer(i) => Some(i.to_string()),
        Yaml::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

fn number(y: &Yaml) -> Option<f64> {
    match y {
        Yaml::Integer(i) => Some(*i as f64),
        Yaml::Real(s) | Yaml::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn strings(y: &Yaml) -> Vec<String> {
    match y {
        Yaml::Array(list) => list.iter().filter_map(string).collect(),
        other => string(other).into_iter().collect(),
    }
}

/// A component value for the `/give` syntax: SNBT as written, or a bare word or id quoted.
fn component_value(y: &Yaml) -> Option<String> {
    let s = string(y)?;
    let t = s.trim();
    let snbt = t.starts_with(['{', '[', '"', '\''])
        || t.parse::<f64>().is_ok()
        || t.trim_end_matches(['b', 's', 'l', 'f', 'd', 'B', 'S', 'L', 'F', 'D'])
            .parse::<f64>()
            .is_ok()
        || t == "true"
        || t == "false";
    Some(if snbt {
        t.to_owned()
    } else {
        format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""))
    })
}

/// Reads the front matter's YAML. An error leaves the page without metadata (spec 3.6).
pub fn parse(yaml: &str) -> Result<FrontMatter, String> {
    let docs = YamlLoader::load_from_str(yaml).map_err(|e| e.to_string())?;
    let Some(doc) = docs.into_iter().next() else {
        return Ok(FrontMatter::default());
    };
    if matches!(doc, Yaml::Null) {
        return Ok(FrontMatter::default());
    }
    if !matches!(doc, Yaml::Hash(_)) {
        return Err("front matter is not a map".into());
    }
    let mut front = FrontMatter {
        item_ids: strings(&doc["item_ids"]),
        categories: strings(&doc["categories"]),
        requires_mods: strings(&doc["requires_mods"]),
        ..FrontMatter::default()
    };
    let nav = &doc["navigation"];
    if let Yaml::Hash(_) = nav {
        front.navigation = true;
        front.title = string(&nav["title"]);
        front.icon = string(&nav["icon"]);
        front.parent = string(&nav["parent"]).filter(|p| !p.is_empty());
        front.position = number(&nav["position"]);
        front.max_depth = number(&nav["max_depth"]).map(|d| d.max(0.0) as u32);
        if let Yaml::Hash(map) = &nav["icon_components"] {
            front.icon_components = map
                .iter()
                .filter_map(|(k, v)| Some(format!("{}={}", string(k)?, component_value(v)?)))
                .collect::<Vec<_>>()
                .join(",");
        }
        // `icon_nbt` (1.20 pages) is ignored on 26.3 (Q-006).
    }
    Ok(front)
}
