//! A `GameScene`'s content (guides spec 6): its children run in document order into blocks, entities, a camera
//! and annotations, which the game then draws with its scene drawing.

use super::Problem;
use super::markdown::{Attrs, Block, Tag};
use super::structure::{State, Structure};
use std::collections::BTreeMap;
use sunrise_core::nbt::Tag as Nbt;

/// The scene drawing's limits (living scenes): more is refused, not cut (spec 6.9).
pub const MAX_BLOCKS: usize = 4096;
pub const MAX_ENTITIES: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub struct SceneEntity {
    pub id: String,
    pub pos: [f64; 3],
    /// Degrees about the vertical axis, and the head's pitch (Q-034).
    pub yaw: f32,
    pub pitch: f32,
    /// `/summon`'s data, SNBT, or "".
    pub data: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// An outline of the box `min`..`max` (exclusive), `thickness` in blocks.
    Box {
        min: [f32; 3],
        max: [f32; 3],
        thickness: f32,
    },
    Line {
        from: [f32; 3],
        to: [f32; 3],
        thickness: f32,
    },
    Diamond {
        pos: [f32; 3],
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    pub shape: Shape,
    pub color: u32,
    pub on_top: bool,
    /// Shown as a tooltip when the pointer is over it; none for an annotation without children.
    pub tooltip: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneModel {
    pub blocks: BTreeMap<[i32; 3], (State, Option<Nbt>)>,
    pub entities: Vec<SceneEntity>,
    /// Yaw, pitch and roll in degrees (spec 6.6.1, 6.6.2).
    pub camera: [f32; 3],
    pub zoom: f32,
    /// `0xAARRGGBB`; 0 is transparent.
    pub background: u32,
    pub interactive: bool,
    pub annotations: Vec<Annotation>,
    /// Problems: unreadable structures, a limit passed (then nothing is drawn: [`SceneModel::refused`]).
    pub errors: Vec<Problem>,
}

fn coords(text: &str) -> Option<[f32; 3]> {
    let v: Vec<f32> = text
        .split_whitespace()
        .map(|s| s.parse().ok())
        .collect::<Option<_>>()?;
    (v.len() == 3 && v.iter().all(|x| x.is_finite())).then(|| [v[0], v[1], v[2]])
}

fn int_attr(a: &Attrs, name: &str, default: i32) -> i32 {
    a.num(name).map_or(default, |v| v.floor() as i32)
}

/// `#RRGGBB` or `#AARRGGBB` to `0xAARRGGBB`.
pub fn color(text: &str) -> Option<u32> {
    let hex = text.trim().strip_prefix('#')?;
    let v = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some(0xff00_0000 | v),
        8 => Some(v),
        _ => None,
    }
}

impl SceneModel {
    /// Builds a scene from a `GameScene` tag. `namespace` is the page's (for ids without one), `read` reads a
    /// structure file written in `src` (none when it is missing).
    pub fn build(
        tag: &Tag<Block>,
        namespace: &str,
        read: &dyn Fn(&str) -> Option<Vec<u8>>,
    ) -> SceneModel {
        let a = &tag.attrs;
        let background = match a.str("background").map(str::trim) {
            None | Some("transparent") => 0,
            // Q-028: `#RRGGBB`/`#AARRGGBB`; anything else is transparent.
            Some(c) => color(c).unwrap_or(0),
        };
        let mut scene = SceneModel {
            blocks: BTreeMap::new(),
            entities: Vec::new(),
            camera: [225.0, 30.0, 0.0],
            zoom: a.num("zoom").filter(|z| *z > 0.0).unwrap_or(1.0),
            background,
            interactive: a.flag("interactive").unwrap_or(false),
            annotations: Vec::new(),
            errors: Vec::new(),
        };
        for child in &tag.children {
            scene.child(child, namespace, read);
        }
        if scene.blocks.len() > MAX_BLOCKS {
            let n = scene.blocks.len().to_string();
            scene
                .errors
                .push(Problem::new("sunrise.guide.error.too_many_blocks", n));
        }
        if scene.entities.len() > MAX_ENTITIES {
            let n = scene.entities.len().to_string();
            scene
                .errors
                .push(Problem::new("sunrise.guide.error.too_many_entities", n));
        }
        scene
    }

    fn child(&mut self, block: &Block, ns: &str, read: &dyn Fn(&str) -> Option<Vec<u8>>) {
        let Block::Tag(tag) = block else {
            // Text between scene tags is not drawn.
            return;
        };
        let a = &tag.attrs;
        let id = |name: &str| a.str(name).map(|i| super::library::qualify(i.trim(), ns));
        match tag.name.as_str() {
            "ImportStructure" => {
                let Some(src) = a.str("src") else {
                    self.errors.push(Problem::new(
                        "sunrise.guide.error.no_attribute",
                        "ImportStructure src",
                    ));
                    return;
                };
                let offset = a
                    .str("pos")
                    .and_then(coords)
                    .map_or([0; 3], |p| p.map(|v| v.floor() as i32));
                let Some(bytes) = read(src) else {
                    self.errors
                        .push(Problem::new("sunrise.guide.error.missing_structure", src));
                    return;
                };
                match Structure::read(&bytes, src.ends_with(".snbt")) {
                    Ok(s) => {
                        for b in s.blocks {
                            let pos = [
                                b.pos[0] + offset[0],
                                b.pos[1] + offset[1],
                                b.pos[2] + offset[2],
                            ];
                            if b.state.name == "minecraft:air" {
                                self.blocks.remove(&pos);
                            } else {
                                self.blocks.insert(pos, (b.state, b.nbt));
                            }
                        }
                        for e in s.entities {
                            let Some(id) = e.nbt.get("id").and_then(|t| match t {
                                Nbt::String(s) => Some(s.to_string_lossy()),
                                _ => None,
                            }) else {
                                continue;
                            };
                            self.entities.push(SceneEntity {
                                id,
                                pos: [
                                    e.pos[0] + offset[0] as f64,
                                    e.pos[1] + offset[1] as f64,
                                    e.pos[2] + offset[2] as f64,
                                ],
                                yaw: 0.0,
                                pitch: 0.0,
                                data: sunrise_client_core::snbt::to_snbt(&e.nbt),
                            });
                        }
                    }
                    Err(e) => {
                        log::warn!("guide structure {src}: {e}");
                        self.errors
                            .push(Problem::new("sunrise.guide.error.bad_structure", src));
                    }
                }
            }
            "Block" => {
                let Some(name) = id("id") else {
                    self.errors
                        .push(Problem::new("sunrise.guide.error.no_attribute", "Block id"));
                    return;
                };
                let pos = [
                    int_attr(a, "x", 0),
                    int_attr(a, "y", 0),
                    int_attr(a, "z", 0),
                ];
                if name == "minecraft:air" {
                    self.blocks.remove(&pos);
                    return;
                }
                let mut state = State::new(&name);
                for (k, v) in a.props() {
                    state.set(&k, &v);
                }
                self.blocks.insert(pos, (state, None));
            }
            "RemoveBlocks" | "RemoveBlock" => {
                let Some(name) = id("id") else {
                    return;
                };
                let props = a.props();
                self.blocks.retain(|_, (s, _)| !s.matches(&name, &props));
            }
            "Entity" => {
                let Some(name) = id("id") else {
                    self.errors.push(Problem::new(
                        "sunrise.guide.error.no_attribute",
                        "Entity id",
                    ));
                    return;
                };
                self.entities.push(SceneEntity {
                    id: name,
                    pos: [
                        a.num("x").unwrap_or(0.5) as f64,
                        a.num("y").unwrap_or(0.0) as f64,
                        a.num("z").unwrap_or(0.5) as f64,
                    ],
                    yaw: a.num("rotationY").unwrap_or(-45.0),
                    pitch: a.num("rotationX").unwrap_or(0.0),
                    data: a.str("data").unwrap_or("").to_owned(),
                });
            }
            "IsometricCamera" => {
                self.camera = [
                    a.num("yaw").unwrap_or(0.0),
                    a.num("pitch").unwrap_or(0.0),
                    a.num("roll").unwrap_or(0.0),
                ];
            }
            "BlockAnnotationTemplate" => {
                let Some(name) = id("id") else {
                    return;
                };
                let props = a.props();
                let at: Vec<[i32; 3]> = self
                    .blocks
                    .iter()
                    .filter(|(_, (s, _))| s.matches(&name, &props))
                    .map(|(p, _)| *p)
                    .collect();
                for child in &tag.children {
                    let Block::Tag(t) = child else {
                        continue;
                    };
                    for p in &at {
                        if let Some(mut ann) = annotation(t) {
                            ann.shape = moved(ann.shape, p.map(|v| v as f32));
                            self.annotations.push(ann);
                        }
                    }
                }
            }
            _ => match annotation(tag) {
                Some(ann) => self.annotations.push(ann),
                None => self.errors.push(Problem::new(
                    "sunrise.guide.error.not_scene",
                    tag.name.clone(),
                )),
            },
        }
    }

    /// Whether a limit refuses the whole scene (spec 6.9): its box shows the error instead.
    pub fn refused(&self) -> bool {
        self.errors
            .iter()
            .any(|p| p.key.starts_with("sunrise.guide.error.too_many"))
    }

    /// The blocks' bounds, `min` inclusive and `max` exclusive; a unit box at the origin when empty.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        let points = self
            .blocks
            .keys()
            .flat_map(|p| [p.map(|v| v as f32), p.map(|v| v as f32 + 1.0)])
            .chain(self.entities.iter().map(|e| e.pos.map(|v| v as f32)));
        for p in points {
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        if min[0] > max[0] {
            return ([0.0; 3], [1.0; 3]);
        }
        (min, max)
    }
}

fn moved(shape: Shape, by: [f32; 3]) -> Shape {
    let add = |p: [f32; 3]| [p[0] + by[0], p[1] + by[1], p[2] + by[2]];
    match shape {
        Shape::Box {
            min,
            max,
            thickness,
        } => Shape::Box {
            min: add(min),
            max: add(max),
            thickness,
        },
        Shape::Line {
            from,
            to,
            thickness,
        } => Shape::Line {
            from: add(from),
            to: add(to),
            thickness,
        },
        Shape::Diamond { pos } => Shape::Diamond { pos: add(pos) },
    }
}

/// An annotation tag (spec 6.7), or none for another tag.
fn annotation(tag: &Tag<Block>) -> Option<Annotation> {
    let a = &tag.attrs;
    let thickness = a.num("thickness").unwrap_or(0.5 / 16.0);
    let shape = match tag.name.as_str() {
        "BlockAnnotation" => {
            let p = [
                int_attr(a, "x", 0),
                int_attr(a, "y", 0),
                int_attr(a, "z", 0),
            ]
            .map(|v| v as f32);
            Shape::Box {
                min: p,
                max: p.map(|v| v + 1.0),
                thickness,
            }
        }
        "BoxAnnotation" => Shape::Box {
            min: a.str("min").and_then(coords).unwrap_or([0.0; 3]),
            max: a.str("max").and_then(coords).unwrap_or([1.0; 3]),
            thickness,
        },
        "LineAnnotation" => Shape::Line {
            from: a.str("from").and_then(coords).unwrap_or([0.0; 3]),
            to: a.str("to").and_then(coords).unwrap_or([1.0; 3]),
            thickness,
        },
        "DiamondAnnotation" => Shape::Diamond {
            pos: a.str("pos").and_then(coords).unwrap_or([0.5; 3]),
        },
        _ => return None,
    };
    Some(Annotation {
        shape,
        color: a.str("color").and_then(color).unwrap_or(0xffff_ffff),
        on_top: a.flag("alwaysOnTop").unwrap_or(false),
        tooltip: tag.children.clone(),
    })
}
