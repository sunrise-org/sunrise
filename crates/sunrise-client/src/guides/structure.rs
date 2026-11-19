//! Structure files placed by `ImportStructure` (guides spec 6.2): vanilla's `.nbt` structure templates and the
//! `.snbt` text form, both the `data` + state-string layout and the `blocks` + palette-index one (Q-031).

use sunrise_core::nbt::{self, Tag};

/// A block state as written: `minecraft:furnace` and its properties.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub name: String,
    pub props: Vec<(String, String)>,
}

impl State {
    pub fn new(name: &str) -> Self {
        State {
            name: name.to_owned(),
            props: Vec::new(),
        }
    }
    /// The command form, `minecraft:furnace[facing=north,lit=true]`.
    pub fn command(&self) -> String {
        if self.props.is_empty() {
            return self.name.clone();
        }
        let props: Vec<String> = self.props.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{}[{}]", self.name, props.join(","))
    }
    /// Whether every given property has the given value (spec 6.5, 6.7's templates).
    pub fn matches(&self, name: &str, props: &[(String, String)]) -> bool {
        self.name == name
            && props
                .iter()
                .all(|(k, v)| self.props.iter().any(|(pk, pv)| pk == k && pv == v))
    }
    /// Sets a property, replacing its value.
    pub fn set(&mut self, key: &str, value: &str) {
        match self.props.iter_mut().find(|(k, _)| k == key) {
            Some(p) => p.1 = value.to_owned(),
            None => self.props.push((key.to_owned(), value.to_owned())),
        }
    }

    /// `ae2:cable_bus{light_level:9,waterlogged:false}` (the SNBT files' form) or `minecraft:stone[axis=y]`.
    pub fn parse(text: &str) -> Option<State> {
        let text = text.trim();
        let (name, props) = match text.find(['{', '[']) {
            Some(i) => {
                let close = if text.as_bytes()[i] == b'{' { '}' } else { ']' };
                let inner = text[i + 1..].strip_suffix(close)?;
                let sep = if close == '}' { ':' } else { '=' };
                let props = inner
                    .split(',')
                    .filter(|p| !p.trim().is_empty())
                    .map(|p| {
                        let (k, v) = p.split_once(sep)?;
                        let unquote = |s: &str| s.trim().trim_matches('"').to_owned();
                        Some((unquote(k), unquote(v)))
                    })
                    .collect::<Option<Vec<_>>>()?;
                (&text[..i], props)
            }
            None => (text, Vec::new()),
        };
        if name.is_empty() {
            return None;
        }
        let name = if name.contains(':') {
            name.to_owned()
        } else {
            format!("minecraft:{name}")
        };
        Some(State { name, props })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StructureBlock {
    pub pos: [i32; 3],
    pub state: State,
    /// Block entity data, without x/y/z.
    pub nbt: Option<Tag>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StructureEntity {
    pub pos: [f64; 3],
    pub nbt: Tag,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Structure {
    pub data_version: Option<i32>,
    pub size: [i32; 3],
    pub blocks: Vec<StructureBlock>,
    pub entities: Vec<StructureEntity>,
}

/// The most a structure file may hold before it is refused (well past a scene's 4096 blocks).
const MAX_FILE: usize = 16 << 20;

fn int(t: &Tag) -> Option<i64> {
    Some(match t {
        Tag::Byte(v) => *v as i64,
        Tag::Short(v) => *v as i64,
        Tag::Int(v) => *v as i64,
        Tag::Long(v) => *v,
        _ => return None,
    })
}

fn float(t: &Tag) -> Option<f64> {
    Some(match t {
        Tag::Float(bits) => f32::from_bits(*bits) as f64,
        Tag::Double(bits) => f64::from_bits(*bits),
        other => int(other)? as f64,
    })
}

fn list(t: Option<&Tag>) -> &[Tag] {
    match t {
        Some(Tag::List(l)) => l,
        _ => &[],
    }
}

fn ints3(t: Option<&Tag>) -> Option<[i32; 3]> {
    match t? {
        Tag::IntArray(v) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        Tag::List(v) if v.len() == 3 => {
            Some([int(&v[0])? as i32, int(&v[1])? as i32, int(&v[2])? as i32])
        }
        _ => None,
    }
}

fn doubles3(t: Option<&Tag>) -> Option<[f64; 3]> {
    match t? {
        Tag::List(v) if v.len() == 3 => Some([float(&v[0])?, float(&v[1])?, float(&v[2])?]),
        Tag::IntArray(v) if v.len() == 3 => Some([v[0] as f64, v[1] as f64, v[2] as f64]),
        _ => None,
    }
}

fn text(t: &Tag) -> Option<String> {
    match t {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// A palette entry: a `{Name, Properties}` compound or a state string.
fn palette_state(t: &Tag) -> Option<State> {
    if let Some(s) = text(t) {
        return State::parse(&s);
    }
    let name = text(t.get("Name")?)?;
    let mut state = State::parse(&name)?;
    if let Some(Tag::Compound(props)) = t.get("Properties") {
        for (k, v) in props {
            state.set(&k.to_string_lossy(), &text(v).unwrap_or_default());
        }
    }
    Some(state)
}

impl Structure {
    /// Reads a structure file; `.snbt` files are text, others gzip-compressed or plain binary NBT.
    pub fn read(bytes: &[u8], snbt: bool) -> Result<Structure, String> {
        if bytes.len() > MAX_FILE {
            return Err("structure file too large".into());
        }
        let root = if snbt {
            let text =
                std::str::from_utf8(bytes).map_err(|_| "structure file is not UTF-8".to_owned())?;
            let text = text.trim_start_matches('\u{feff}');
            nbt::parse_snbt(text.trim()).map(|(t, _)| t)?
        } else {
            let raw = if bytes.starts_with(&[0x1f, 0x8b]) {
                use std::io::Read;
                let mut out = Vec::new();
                flate2::read::GzDecoder::new(bytes)
                    .take(MAX_FILE as u64 * 4)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("structure file: {e}"))?;
                out
            } else {
                bytes.to_vec()
            };
            nbt::read_named(&raw, nbt::Limits::templates())
                .map_err(|e| format!("structure file: {e:?}"))?
                .0
                .tag
        };
        Self::from_tag(&root)
    }

    pub fn from_tag(root: &Tag) -> Result<Structure, String> {
        if !matches!(root, Tag::Compound(_)) {
            return Err("structure file is not a compound".into());
        }
        let size = ints3(root.get("size")).unwrap_or([0; 3]);
        // `palettes` holds several: the first is shown (Q-030).
        let palette: Vec<Option<State>> = match root.get("palette") {
            Some(Tag::List(p)) => p.iter().map(palette_state).collect(),
            _ => list(root.get("palettes"))
                .first()
                .map(|p| list(Some(p)).iter().map(palette_state).collect())
                .unwrap_or_default(),
        };
        let mut blocks = Vec::new();
        let entries = list(root.get("blocks"))
            .iter()
            .chain(list(root.get("data")));
        for b in entries {
            let Some(pos) = ints3(b.get("pos")) else {
                continue;
            };
            let state = match b.get("state") {
                Some(Tag::String(s)) => State::parse(&s.to_string_lossy()),
                Some(t) => int(t).and_then(|i| palette.get(i as usize).cloned().flatten()),
                None => None,
            };
            let Some(state) = state else {
                continue;
            };
            blocks.push(StructureBlock {
                pos,
                state,
                nbt: b.get("nbt").cloned(),
            });
        }
        let entities = list(root.get("entities"))
            .iter()
            .filter_map(|e| {
                Some(StructureEntity {
                    pos: doubles3(e.get("pos"))?,
                    nbt: e.get("nbt")?.clone(),
                })
            })
            .collect();
        Ok(Structure {
            data_version: root.get("DataVersion").and_then(int).map(|v| v as i32),
            size,
            blocks,
            entities,
        })
    }
}
