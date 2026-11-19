//! Every guide found in the loaded resources (guides spec sections 2, 3, 7, 8, 9.2 and 9.4): the pages in the
//! player's language, their front matter and parsed bodies, the contents tree, item lookup, links and assets.

use super::front::{self, FrontMatter};
use super::markdown::{self, Document};
use std::collections::{BTreeMap, HashMap, HashSet};

/// What the library is built from: the loaded resources' names (`assets/<ns>/...`, the winning copy of each)
/// and a way to read one, the language, which mods are installed and their display names.
pub struct Sources<'a> {
    pub names: &'a [String],
    pub read: &'a dyn Fn(&str) -> Option<Vec<u8>>,
    /// The game's language code, `en_us`.
    pub language: &'a str,
    /// Whether a mod id is installed (spec 7.2).
    pub installed: &'a dyn Fn(&str) -> bool,
    /// A namespace's mod display name (spec 2.3.2).
    pub mod_name: &'a dyn Fn(&str) -> Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Guide {
    /// `<ns>:<name>`; `<ns>:guide` for an `ae2guide/` folder (Q-001).
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub default_language: String,
    /// `custom_colors`: id (`<ns>:<name>`) to its dark- and light-mode colours (`0xRRGGBB`).
    pub colors: HashMap<String, (u32, u32)>,
    /// The top of its contents tree, in order.
    pub roots: Vec<usize>,
    /// Its files that are not pages: path under the guide root to the resource, per language (`""` the default).
    assets: HashMap<String, BTreeMap<String, String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// `<ns>:<path>.md`.
    pub id: String,
    pub guide: usize,
    /// The path under the guide root.
    pub path: String,
    /// The resource the page was read from (a translation when one is used).
    pub resource: String,
    pub front: FrontMatter,
    pub doc: Document,
    pub title: String,
    /// The mod whose absence hides the page (spec 7.3), `!id` for one whose presence does.
    pub hidden: Option<String>,
    /// Its parent and children in the contents tree, children in order.
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// The page's icon: item id and components.
    pub icon: Option<(String, String)>,
    /// Problems found while reading it (spec 4.13).
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Library {
    pub guides: Vec<Guide>,
    pub pages: Vec<Page>,
    pub language: String,
    by_id: HashMap<String, usize>,
    by_item: HashMap<String, usize>,
    /// Problems with the guides as a whole (unreadable files, missing parents, cycles).
    pub errors: Vec<String>,
}

/// Where a link leads (spec 5.1).
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Page {
        page: usize,
        anchor: Option<String>,
    },
    External(String),
    /// A page that is not there: its id.
    Missing(String),
    /// A page hidden by its required mods (Q-015): the page and the mod.
    Hidden {
        page: usize,
        mod_id: String,
    },
}

/// One file of a guide, before reading.
struct Found {
    guide: String,
    namespace: String,
    /// Under the guide root, without the language folder.
    path: String,
    /// `""` for the default language.
    lang: String,
    resource: String,
    /// Which copy wins when two files give the same page: lower first (Q-002, Q-005).
    rank: (u8, String),
}

/// A resource path as Minecraft accepts it (spec 2.1.3).
fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
}

/// A file name to the guide it belongs to, or none.
fn classify(name: &str) -> Option<Found> {
    let rest = name.strip_prefix("assets/")?;
    let (ns, rest) = rest.split_once('/')?;
    let (guide, gns, path, rank) = if let Some(path) = rest.strip_prefix("ae2guide/") {
        (format!("{ns}:guide"), ns, path, (1, ns.to_owned()))
    } else {
        let rest = rest.strip_prefix("guides/")?;
        let (gns, rest) = rest.split_once('/')?;
        let (gpath, path) = rest.split_once('/')?;
        // The guide's own namespace's copy first, then by namespace (Q-005).
        let rank = (
            0,
            if ns == gns {
                String::new()
            } else {
                ns.to_owned()
            },
        );
        (format!("{gns}:{gpath}"), gns, path, rank)
    };
    let (lang, path) = match path.split_once('/') {
        Some((folder, inner)) if folder.len() > 1 && folder.starts_with('_') => {
            (folder[1..].to_owned(), inner)
        }
        _ => (String::new(), path),
    };
    Some(Found {
        guide,
        namespace: gns.to_owned(),
        path: path.to_owned(),
        lang,
        resource: name.to_owned(),
        rank,
    })
}

/// `a/b/../c/./d` to `a/c/d`; none when it climbs out of the root.
pub fn normalize(path: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            p => out.push(p),
        }
    }
    Some(out.join("/"))
}

fn parse_color(s: &str) -> Option<u32> {
    let hex = s.trim().strip_prefix('#')?;
    (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok())?
}

/// An id with the page's namespace when it has none (spec 4.8).
pub fn qualify(id: &str, namespace: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("{namespace}:{id}")
    }
}

/// Whether a requirement (`id` or `!id`) holds.
pub fn requirement_met(req: &str, installed: &dyn Fn(&str) -> bool) -> bool {
    let req = req.trim();
    match req.strip_prefix('!') {
        Some(absent) => !is_installed(absent.trim(), installed),
        None => is_installed(req, installed),
    }
}

fn is_installed(id: &str, installed: &dyn Fn(&str) -> bool) -> bool {
    id == "minecraft" || installed(id)
}

impl Library {
    pub fn build(src: &Sources<'_>) -> Self {
        let mut lib = Library {
            language: src.language.to_owned(),
            ..Library::default()
        };
        let mut files: BTreeMap<String, Vec<Found>> = BTreeMap::new();
        let mut definitions: HashMap<String, serde_json::Value> = HashMap::new();
        for name in src.names {
            if let Some(rest) = name.strip_prefix("assets/")
                && let Some((ns, file)) = rest.split_once("/guideme_guides/")
                && let Some(stem) = file.strip_suffix(".json")
                && !stem.contains('/')
            {
                match (src.read)(name).and_then(|b| serde_json::from_slice(&b).ok()) {
                    Some(json) => {
                        definitions.insert(format!("{ns}:{stem}"), json);
                    }
                    None => lib
                        .errors
                        .push(format!("{name}: not a readable guide definition")),
                }
                continue;
            }
            let Some(found) = classify(name) else {
                continue;
            };
            if !valid_path(&found.path) {
                log::warn!("guide file {name} is not a valid resource path; skipped");
                lib.errors
                    .push(format!("{name}: not a valid resource path"));
                continue;
            }
            files.entry(found.guide.clone()).or_default().push(found);
        }
        for (guide_id, mut found) in files {
            found.sort_by(|a, b| a.rank.cmp(&b.rank));
            let namespace = found[0].namespace.clone();
            let def = definitions.get(&guide_id);
            let default_language = def
                .and_then(|d| d.get("default_language")?.as_str())
                .unwrap_or("en_us")
                .to_owned();
            let colors = def
                .and_then(|d| d.get("custom_colors")?.as_object())
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| {
                            let dark = parse_color(v.get("dark_mode")?.as_str()?)?;
                            let light = parse_color(v.get("light_mode")?.as_str()?).unwrap_or(dark);
                            Some((qualify(k, &namespace), (dark, light)))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let name = (src.mod_name)(&namespace).unwrap_or_else(|| namespace.clone());
            let g = lib.guides.len();
            let mut assets: HashMap<String, BTreeMap<String, String>> = HashMap::new();
            // path -> (default copy, this language's copy); the first by rank wins.
            let mut pages: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();
            let wanted = if src.language == default_language {
                ""
            } else {
                src.language
            };
            for f in found {
                if !f.path.ends_with(".md") {
                    assets
                        .entry(f.path)
                        .or_default()
                        .entry(f.lang)
                        .or_insert(f.resource);
                    continue;
                }
                let entry = pages.entry(f.path).or_default();
                if f.lang.is_empty() {
                    entry.0.get_or_insert(f.resource);
                } else if f.lang == wanted {
                    entry.1.get_or_insert(f.resource);
                }
            }
            lib.guides.push(Guide {
                id: guide_id,
                namespace: namespace.clone(),
                name,
                default_language,
                colors,
                roots: Vec::new(),
                assets,
            });
            for (path, (default, translated)) in pages {
                // A translation without a default page is ignored (Q-041).
                let Some(default) = default else {
                    continue;
                };
                let resource = translated.unwrap_or(default);
                let Some(bytes) = (src.read)(&resource) else {
                    lib.errors.push(format!("{resource}: could not be read"));
                    continue;
                };
                let Ok(text) = String::from_utf8(bytes) else {
                    log::warn!("guide page {resource} is not UTF-8; skipped");
                    lib.errors.push(format!("{resource}: not UTF-8"));
                    continue;
                };
                let page = Page::read(g, &namespace, &path, &resource, &text, src.installed);
                lib.by_id.entry(page.id.clone()).or_insert(lib.pages.len());
                lib.pages.push(page);
            }
        }
        lib.index_items();
        lib.build_tree();
        lib
    }

    fn index_items(&mut self) {
        // Several pages claiming an item: the page of the guide whose namespace is the item's, then the first
        // by page id (Q-047).
        let mut claims: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, p) in self.pages.iter().enumerate() {
            if p.hidden.is_some() {
                continue;
            }
            for item in &p.front.item_ids {
                claims
                    .entry(qualify(item, &self.guides[p.guide].namespace))
                    .or_default()
                    .push(i);
            }
        }
        for (item, mut pages) in claims {
            let ns = item.split(':').next().unwrap_or("").to_owned();
            pages.sort_by_key(|&i| {
                (
                    self.guides[self.pages[i].guide].namespace != ns,
                    self.pages[i].id.clone(),
                )
            });
            self.by_item.insert(item, pages[0]);
        }
    }

    fn build_tree(&mut self) {
        let n = self.pages.len();
        let mut parent = vec![None; n];
        for i in 0..n {
            let p = &self.pages[i];
            if !p.front.navigation || p.hidden.is_some() {
                continue;
            }
            let Some(written) = p.front.parent.clone() else {
                continue;
            };
            match self.page_by_ref(i, &written) {
                Some(j)
                    if self.pages[j].front.navigation
                        && self.pages[j].hidden.is_none()
                        && j != i =>
                {
                    parent[i] = Some(j);
                }
                _ => {
                    let msg = format!("{}: parent {written} not found", p.id);
                    self.pages[i].errors.push(msg.clone());
                    self.errors.push(msg);
                }
            }
        }
        // Cycles: walking up from a page must reach a root; the link that closes a loop is cut.
        for i in 0..n {
            let mut seen = HashSet::new();
            let mut at = i;
            seen.insert(at);
            while let Some(up) = parent[at] {
                if !seen.insert(up) {
                    let msg = format!("{}: its parent makes a cycle", self.pages[at].id);
                    self.errors.push(msg.clone());
                    self.pages[at].errors.push(msg);
                    parent[at] = None;
                    break;
                }
                at = up;
            }
        }
        let key = |lib: &Library, i: usize| {
            let p = &lib.pages[i];
            // A page without `position` sorts as 0 (Q-045).
            (
                p.front.position.unwrap_or(0.0),
                p.title.to_lowercase(),
                p.id.clone(),
            )
        };
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut roots: Vec<Vec<usize>> = vec![Vec::new(); self.guides.len()];
        for i in 0..n {
            let p = &self.pages[i];
            if !p.front.navigation || p.hidden.is_some() {
                continue;
            }
            match parent[i] {
                Some(up) => children[up].push(i),
                None => roots[p.guide].push(i),
            }
        }
        let sort = |list: &mut Vec<usize>, lib: &Library| {
            list.sort_by(|&a, &b| {
                key(lib, a)
                    .partial_cmp(&key(lib, b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        };
        for list in children.iter_mut().chain(roots.iter_mut()) {
            sort(list, self);
        }
        for (i, kids) in children.into_iter().enumerate() {
            self.pages[i].parent = parent[i];
            self.pages[i].children = kids;
        }
        for (g, list) in roots.into_iter().enumerate() {
            self.guides[g].roots = list;
        }
    }

    /// A page named in front matter: a full id, or a path under the page's guide root (spec 3.3).
    fn page_by_ref(&self, from: usize, written: &str) -> Option<usize> {
        if written.contains(':') {
            let (ns, path) = written.split_once(':')?;
            return self
                .by_id
                .get(&format!("{ns}:{}", normalize(path)?))
                .copied();
        }
        let path = normalize(written)?;
        let guide = self.pages[from].guide;
        self.pages
            .iter()
            .position(|p| p.guide == guide && p.path == path)
    }

    pub fn page(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }

    /// An item's page (spec 9.4); hidden pages are never one.
    pub fn item_page(&self, item: &str) -> Option<usize> {
        self.by_item.get(item).copied()
    }

    /// Whether the item has a page claiming it.
    pub fn has_item(&self, item: &str) -> bool {
        self.by_item.contains_key(item)
    }

    /// Resolves a link written on page `from` (spec 5.1).
    pub fn resolve(&self, from: usize, target: &str) -> Target {
        let target = target.trim();
        if target.starts_with("http://") || target.starts_with("https://") {
            return Target::External(target.to_owned());
        }
        let (path, anchor) = match target.split_once('#') {
            Some((p, a)) => (p, Some(a.to_owned()).filter(|a| !a.is_empty())),
            None => (target, None),
        };
        let page = &self.pages[from];
        if path.is_empty() {
            return Target::Page { page: from, anchor };
        }
        let (id, found) = if let Some((ns, p)) = path.split_once(':') {
            let id = format!("{ns}:{}", normalize(p).unwrap_or_else(|| p.to_owned()));
            let found = self.by_id.get(&id).copied();
            (id, found)
        } else {
            let dir = page.path.rsplit_once('/').map_or("", |(d, _)| d);
            let joined = normalize(&format!("{dir}/{path}"));
            let ns = &self.guides[page.guide].namespace;
            let found = joined.as_ref().and_then(|j| {
                self.pages
                    .iter()
                    .position(|p| p.guide == page.guide && &p.path == j)
                    .or_else(|| self.by_id.get(&format!("{ns}:{j}")).copied())
            });
            (
                format!("{ns}:{}", joined.unwrap_or_else(|| path.to_owned())),
                found,
            )
        };
        match found {
            Some(p) => match &self.pages[p].hidden {
                Some(m) => Target::Hidden {
                    page: p,
                    mod_id: m.clone(),
                },
                None => Target::Page { page: p, anchor },
            },
            None => Target::Missing(id),
        }
    }

    /// A guide asset (image, structure) written on page `from`: the resource to read, the player's language's
    /// copy first (spec 5.4.2, 8.2; Q-029).
    pub fn asset(&self, from: usize, src: &str) -> Option<String> {
        let page = &self.pages[from];
        let guide = &self.guides[page.guide];
        let path = if let Some((_, p)) = src.split_once(':') {
            normalize(p)?
        } else {
            let dir = page.path.rsplit_once('/').map_or("", |(d, _)| d);
            normalize(&format!("{dir}/{src}"))?
        };
        let copies = guide.assets.get(&path)?;
        copies
            .get(&self.language)
            .or_else(|| copies.get(""))
            .or_else(|| copies.values().next())
            .cloned()
    }

    /// Pages listed by `CategoryIndex` (spec 5.8): exact category names (Q-022), by title (Q-023).
    pub fn category(&self, name: &str) -> Vec<usize> {
        let mut list: Vec<usize> = (0..self.pages.len())
            .filter(|&i| {
                self.pages[i].hidden.is_none()
                    && self.pages[i].front.categories.iter().any(|c| c == name)
            })
            .collect();
        list.sort_by_key(|&i| (self.pages[i].title.to_lowercase(), self.pages[i].id.clone()));
        list
    }

    /// The sidebar's rows (spec 9.2): each guide, then its tree within the depth limits (spec 9.2.3), entries
    /// whose key (`guide id` or page id) is in `open` showing their children.
    pub fn rows(&self, open: &HashSet<String>) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut guides: Vec<usize> = (0..self.guides.len())
            .filter(|&g| !self.guides[g].roots.is_empty())
            .collect();
        guides.sort_by_key(|&g| self.guides[g].name.to_lowercase());
        for g in guides {
            let guide = &self.guides[g];
            let is_open = open.contains(&guide.id);
            rows.push(Row {
                entry: Entry::Guide(g),
                depth: 0,
                has_children: true,
                open: is_open,
            });
            if is_open {
                for &p in &guide.roots {
                    self.page_rows(p, 1, 2, open, &mut rows);
                }
            }
        }
        rows
    }

    fn page_rows(
        &self,
        p: usize,
        depth: usize,
        left: u32,
        open: &HashSet<String>,
        rows: &mut Vec<Row>,
    ) {
        if left == 0 {
            return;
        }
        let page = &self.pages[p];
        // A page's `max_depth` replaces the levels its children may use.
        let below = page.front.max_depth.unwrap_or(left - 1);
        let has_children = below > 0 && !page.children.is_empty();
        let is_open = has_children && open.contains(&page.id);
        rows.push(Row {
            entry: Entry::Page(p),
            depth,
            has_children,
            open: is_open,
        });
        if is_open {
            for &c in &page.children {
                self.page_rows(c, depth + 1, below, open, rows);
            }
        }
    }

    /// The keys to open so that page `p` shows in the sidebar: its guide and ancestors.
    pub fn path_keys(&self, p: usize) -> Vec<String> {
        let mut keys = Vec::new();
        let mut at = self.pages[p].parent;
        let mut root = p;
        while let Some(up) = at {
            keys.push(self.pages[up].id.clone());
            root = up;
            at = self.pages[up].parent;
        }
        keys.push(self.guides[self.pages[root].guide].id.clone());
        keys
    }
}

/// A sidebar row.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub entry: Entry,
    pub depth: usize,
    pub has_children: bool,
    pub open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Guide(usize),
    Page(usize),
}

impl Page {
    fn read(
        guide: usize,
        namespace: &str,
        path: &str,
        resource: &str,
        text: &str,
        installed: &dyn Fn(&str) -> bool,
    ) -> Page {
        let (yaml, body) = front::split(text);
        let mut errors = Vec::new();
        let front = match yaml.map(front::parse) {
            Some(Ok(f)) => f,
            Some(Err(e)) => {
                log::warn!("guide page {resource}: front matter: {e}");
                errors.push(format!("front matter: {e}"));
                FrontMatter::default()
            }
            None => FrontMatter::default(),
        };
        let doc = markdown::parse(body);
        let file = path.rsplit('/').next().unwrap_or(path);
        // The tree's title, else the first level-1 heading, else the file name (Q-008).
        let title = front
            .title
            .clone()
            .or_else(|| doc.title())
            .unwrap_or_else(|| file.trim_end_matches(".md").to_owned());
        let hidden = front
            .requires_mods
            .iter()
            .find(|r| !requirement_met(r, installed))
            .map(|r| r.trim().to_owned());
        let icon = front
            .icon
            .as_ref()
            .map(|i| (qualify(i, namespace), front.icon_components.clone()));
        Page {
            id: format!("{namespace}:{path}"),
            guide,
            path: path.to_owned(),
            resource: resource.to_owned(),
            front,
            doc,
            title,
            hidden,
            parent: None,
            children: Vec::new(),
            icon,
            errors,
        }
    }
}
