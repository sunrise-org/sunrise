//! What a composite-style pass's draw provably rewrites (#6858): whether its vertex stage covers
//! every pixel of its buffers, and whether its fragment stage can leave a covered pixel unwritten.
//!
//! [`covers_screen`] runs the translated vertex stage at load, once for each vertex of the quad
//! draw ([`QUAD_VERTICES`]), on what is known then: literals, constants and the vertex index. A
//! uniform, a texture or any value it can't follow is unknown, and so is every float result
//! that isn't exactly representable: a result that needs no rounding is the same bits however a
//! GPU rounds or fuses the operations that make it. A branch on an unknown condition, a loop, a
//! switch and a function's second call aren't run: what they may store becomes unknown. Each
//! function body is run at most once per vertex, so the work follows the module's size.
//!
//! Anything unknown in the positions leaves the pass unproven, which keeps the frame-start copy.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use naga::{
    AddressSpace, ArraySize, BinaryOperator, Binding, Block, BuiltIn, Expression, Function,
    GlobalVariable, Handle, Literal, LocalVariable, MathFunction, Module, RelationalFunction,
    ScalarKind, ShaderStage, Statement, Type, TypeInner, UnaryOperator,
};

/// The vertices of the quad a composite-style pass draws, a triangle list
/// (`draw(0..QUAD_VERTICES, 0..1)`): the translator's six `hz_quad` corners.
pub const QUAD_VERTICES: u32 = 6;

/// Value nesting the interpreter follows: the translator's deepest composite input,
/// `hz_TextureMatrix` (an array of matrices of vectors), inside one struct. A deeper value is
/// unknown, which only keeps the copy; it also bounds the interpreter's recursion on values.
const MAX_DEPTH: u32 = 4;

/// How far outside the screen (-1..1) a quad's vertex may lie, in clip space. The containment
/// below is exact, but a GPU clips and sets up triangles in f32: at 2^24, neighbouring f32
/// values are a whole unit apart, so one rounding there can move an edge across the screen's
/// corner pixels. Within ±4 they are 2^-21 apart (under 5e-7), while a pixel centre lies half a
/// pixel inside the screen's edge, 1/W in clip space for a buffer W pixels wide: 1.5e-5 even
/// for W = 65536, some 30 of those steps. ±4 holds the usual full-screen triangle, whose
/// corners are at -1 and 3.
const CLIP_LIMIT: f32 = 4.0;

/// Whether every pixel a pass's fragment stage runs for is written: no `discard` anywhere in the
/// module and no sample mask output.
pub fn writes_every_covered_pixel(fragment: &Module) -> bool {
    let bodies = fragment
        .functions
        .iter()
        .map(|(_, f)| f)
        .chain(fragment.entry_points.iter().map(|e| &e.function));
    let mut blocks: Vec<&Block> = bodies.map(|f| &f.body).collect();
    while let Some(block) = blocks.pop() {
        for statement in block {
            match statement {
                Statement::Kill => return false,
                Statement::Block(b) => blocks.push(b),
                Statement::If { accept, reject, .. } => blocks.extend([accept, reject]),
                Statement::Switch { cases, .. } => blocks.extend(cases.iter().map(|c| &c.body)),
                Statement::Loop {
                    body, continuing, ..
                } => blocks.extend([body, continuing]),
                _ => {}
            }
        }
    }
    !fragment.entry_points.iter().any(|e| {
        outputs(fragment, &e.function).any(|b| matches!(b, Binding::BuiltIn(BuiltIn::SampleMask)))
    })
}

/// The bindings of a function's result: its own, or its struct's members'.
fn outputs<'m>(module: &'m Module, function: &'m Function) -> impl Iterator<Item = &'m Binding> {
    let result = function.result.as_ref();
    let own = result.and_then(|r| r.binding.as_ref());
    let members = result
        .filter(|r| r.binding.is_none())
        .and_then(|r| match &module.types[r.ty].inner {
            TypeInner::Struct { members, .. } => Some(members),
            _ => None,
        })
        .into_iter()
        .flatten()
        .filter_map(|m| m.binding.as_ref());
    own.into_iter().chain(members)
}

/// Whether the vertex stage's quad covers every pixel of its target: each of its two
/// triangles' positions is known exactly, with `w` 1 and `z` within `0..=w`, and the closed
/// square from (-1, -1) to (1, 1) lies inside one triangle, or inside the convex quad two
/// triangles sharing an edge make.
pub fn covers_screen(vertex: &Module) -> bool {
    let Some(entry) = vertex
        .entry_points
        .iter()
        .find(|e| e.stage == ShaderStage::Vertex)
    else {
        return false;
    };
    // A clip distance can cut pixels out of the quad.
    if outputs(vertex, &entry.function).any(|b| {
        matches!(
            b,
            Binding::BuiltIn(BuiltIn::ClipDistances | BuiltIn::CullDistance)
        )
    }) {
        return false;
    }
    let Some(positions) = quad_positions(vertex) else {
        return false;
    };
    let mut corners = Vec::with_capacity(positions.len());
    for p in positions {
        if p[3] != 1.0
            || !(0.0..=p[3]).contains(&p[2])
            || p[0].abs() > CLIP_LIMIT
            || p[1].abs() > CLIP_LIMIT
        {
            return false;
        }
        corners.push([p[0], p[1]]);
    }
    let triangles = [&corners[0..3], &corners[3..6]];
    triangles.iter().any(|t| polygon_covers(t)) || quad_covers(triangles[0], triangles[1])
}

/// The clip-space position of each vertex of the quad draw, when all are known exactly.
fn quad_positions(vertex: &Module) -> Option<Vec<[f32; 4]>> {
    let entry = (vertex.entry_points.iter()).find(|e| e.stage == ShaderStage::Vertex)?;
    let context = Context::new(vertex);
    (0..QUAD_VERTICES)
        .map(|index| context.position(&entry.function, index))
        .collect()
}

/// The corners of the square every pixel centre lies in.
const SQUARE: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

/// Twice the signed area of `a`, `b`, `c`, exactly; `None` past the exact range.
fn orient(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> Option<i8> {
    let d = |p: [f32; 2], q: [f32; 2], i: usize| Exact::of(q[i])?.add(Exact::of(p[i])?.neg()?);
    let left = d(a, b, 0)?.mul(d(a, c, 1)?)?;
    let right = d(a, b, 1)?.mul(d(a, c, 0)?)?;
    Some(left.add(right.neg()?)?.sign())
}

/// Whether the closed convex polygon `points` (at least three, wound one way) holds every
/// corner of [`SQUARE`].
fn polygon_covers(points: &[[f32; 2]]) -> bool {
    let n = points.len();
    let edge = |i: usize, p: [f32; 2]| orient(points[i], points[(i + 1) % n], p);
    let Some(winding) = edge(0, points[2]) else {
        return false;
    };
    if winding == 0 {
        return false;
    }
    // Every turn has the polygon's winding: it is convex.
    let convex = (0..n).all(|i| edge(i, points[(i + 2) % n]) == Some(winding));
    convex
        && SQUARE
            .iter()
            .all(|&corner| (0..n).all(|i| edge(i, corner).is_some_and(|s| s == 0 || s == winding)))
}

/// Two triangles sharing exactly one edge `p`–`q`, with `r` and `s` their other corners:
/// their union is the quad `p r q s` when that quad is convex.
fn quad_covers(a: &[[f32; 2]], b: &[[f32; 2]]) -> bool {
    let shared: Vec<[f32; 2]> = a.iter().filter(|p| b.contains(p)).copied().collect();
    let [p, q] = shared[..] else {
        return false;
    };
    if p == q {
        return false;
    }
    let other = |t: &[[f32; 2]]| t.iter().find(|c| **c != p && **c != q).copied();
    let (Some(r), Some(s)) = (other(a), other(b)) else {
        return false;
    };
    polygon_covers(&[p, r, q, s])
}

/// A dyadic rational `m × 2^e`, computed without rounding.
#[derive(Clone, Copy, Debug)]
struct Exact {
    m: i128,
    e: i32,
}

impl Exact {
    /// A finite f32 that is zero or normal; a subnormal one is left unknown.
    fn of(f: f32) -> Option<Self> {
        if f == 0.0 {
            return Some(Self { m: 0, e: 0 });
        }
        if !f.is_normal() {
            return None;
        }
        let bits = f.to_bits();
        let frac = i128::from((bits & 0x7f_ffff) | 0x80_0000);
        let exponent = i32::try_from((bits >> 23) & 0xff).ok()?;
        Some(Self {
            m: if bits >> 31 == 0 { frac } else { -frac },
            e: exponent - 150,
        })
    }

    fn add(self, other: Self) -> Option<Self> {
        let (high, low) = if self.e >= other.e {
            (self, other)
        } else {
            (other, self)
        };
        let shift = u32::try_from(high.e.checked_sub(low.e)?).ok()?;
        Some(Self {
            m: shl(high.m, shift)?.checked_add(low.m)?,
            e: low.e,
        })
    }

    fn neg(self) -> Option<Self> {
        Some(Self {
            m: self.m.checked_neg()?,
            e: self.e,
        })
    }

    fn mul(self, other: Self) -> Option<Self> {
        Some(Self {
            m: self.m.checked_mul(other.m)?,
            e: self.e.checked_add(other.e)?,
        })
    }

    fn sign(self) -> i8 {
        self.m.signum() as i8
    }

    /// The f32 holding this value exactly: zero or normal.
    fn to_f32(self) -> Option<f32> {
        if self.m == 0 {
            return Some(0.0);
        }
        let zeros = self.m.trailing_zeros();
        let m = self.m >> zeros;
        let e = self.e.checked_add(i32::try_from(zeros).ok()?)?;
        let bits = 128 - m.unsigned_abs().leading_zeros();
        if bits > 24 {
            return None;
        }
        let top = e.checked_add(i32::try_from(bits).ok()? - 1)?;
        if !(-126..=127).contains(&top) {
            return None;
        }
        // At most 24 bits times a power of two within f64's range: exact.
        Some((m as f64 * 2f64.powi(e)) as f32)
    }
}

/// `m << shift`, or `None` when bits or the sign would be lost.
fn shl(m: i128, shift: u32) -> Option<i128> {
    if m == 0 {
        return Some(0);
    }
    if shift >= 126 || m.unsigned_abs().leading_zeros() <= shift + 1 {
        return None;
    }
    Some(m << shift)
}

/// `a + b` when exact.
fn add(a: f32, b: f32) -> Option<f32> {
    Exact::of(a)?.add(Exact::of(b)?)?.to_f32()
}

fn sub(a: f32, b: f32) -> Option<f32> {
    Exact::of(a)?.add(Exact::of(b)?.neg()?)?.to_f32()
}

fn mul(a: f32, b: f32) -> Option<f32> {
    Exact::of(a)?.mul(Exact::of(b)?)?.to_f32()
}

/// `Σ a[i] × b[i]` when every product and every partial sum, in any order, is exact: then the
/// result is the same whatever order the terms are summed in, fused or not.
fn dot(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let products: Vec<Exact> = (a.iter().zip(b))
        .map(|(&x, &y)| {
            let p = Exact::of(x)?.mul(Exact::of(y)?)?;
            p.to_f32().map(|_| p)
        })
        .collect::<Option<_>>()?;
    let mut total = None;
    for subset in 1..1u32 << products.len() {
        let mut sum = Exact { m: 0, e: 0 };
        for (i, p) in products.iter().enumerate() {
            if subset & 1 << i != 0 {
                sum = sum.add(*p)?;
            }
        }
        total = Some(sum.to_f32()?);
    }
    total
}

/// A variable a pointer starts from: a global, or a local of a function (`None`: the entry
/// point), each run at most once per vertex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Root {
    Global(Handle<GlobalVariable>),
    Local(Option<Handle<Function>>, Handle<LocalVariable>),
}

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Unknown,
    F(f32),
    I(i32),
    U(u32),
    B(bool),
    /// A vector's components or a struct's members.
    List(Rc<Vec<Val>>),
    /// A matrix's columns, each a `List`.
    Mat(Rc<Vec<Val>>),
    /// An array of `len`: the elements set, and what every other one holds.
    Array {
        len: u32,
        set: Rc<BTreeMap<u32, Val>>,
        rest: Rc<Val>,
    },
    Ptr(Root, Vec<u32>),
    /// A pointer into a variable at an index not known.
    Somewhere(Root),
}

const UNKNOWN: Val = Val::Unknown;

fn list(items: Vec<Val>) -> Val {
    Val::List(Rc::new(items))
}

fn mat(columns: Vec<Val>) -> Val {
    Val::Mat(Rc::new(columns))
}

impl Val {
    fn element(&self, i: u32) -> Val {
        self.element_ref(i).clone()
    }

    fn element_ref(&self, i: u32) -> &Val {
        match self {
            Val::List(items) | Val::Mat(items) => items.get(i as usize).unwrap_or(&UNKNOWN),
            Val::Array { len, set, rest } if i < *len => set.get(&i).unwrap_or(rest),
            _ => &UNKNOWN,
        }
    }

    /// The value at `path`, walked without recursion and ended at the first unknown part.
    fn at(&self, path: &[u32]) -> Val {
        let mut v = self;
        for &i in path {
            if *v == Val::Unknown {
                break;
            }
            v = v.element_ref(i);
        }
        v.clone()
    }

    fn index(&self) -> Option<u32> {
        match *self {
            Val::I(i) => u32::try_from(i).ok(),
            Val::U(u) => Some(u),
            _ => None,
        }
    }

    fn scalars(&self) -> Option<Vec<f32>> {
        match self {
            Val::List(items) => items
                .iter()
                .map(|v| match v {
                    Val::F(f) => Some(*f),
                    _ => None,
                })
                .collect(),
            _ => None,
        }
    }
}

/// The module as the interpreter reads it: its constant values, nesting depths and the
/// globals each function may store to.
struct Context<'m> {
    module: &'m Module,
    globals: Vec<Val>,
    depths: Vec<u32>,
    /// By function: the globals it or its callees may store to; `None` when not known.
    stores: Vec<Option<BTreeSet<Handle<GlobalVariable>>>>,
    /// The module's size: its expressions, statements, globals and types. Every budget is
    /// this, so the work stays linear in what the pack wrote.
    size: u64,
    /// What is left of this vertex's work: statements run, expressions evaluated, copies.
    budget: Cell<u64>,
}

/// Every expression, statement, global and type of `module`.
fn module_size(module: &Module) -> u64 {
    let functions = (module.functions.iter().map(|(_, f)| f))
        .chain(module.entry_points.iter().map(|e| &e.function));
    let mut n =
        module.global_expressions.len() + module.global_variables.len() + module.types.len();
    for f in functions {
        n += f.expressions.len() + f.local_variables.len();
        let mut blocks = vec![&f.body];
        while let Some(block) = blocks.pop() {
            n += block.len();
            for statement in block {
                match statement {
                    Statement::Block(b) => blocks.push(b),
                    Statement::If { accept, reject, .. } => blocks.extend([accept, reject]),
                    Statement::Switch { cases, .. } => blocks.extend(cases.iter().map(|c| &c.body)),
                    Statement::Loop {
                        body, continuing, ..
                    } => blocks.extend([body, continuing]),
                    _ => {}
                }
            }
        }
    }
    u64::try_from(n).unwrap_or(u64::MAX)
}

struct Frame<'m> {
    id: Option<Handle<Function>>,
    function: &'m Function,
    vals: Vec<Option<Val>>,
    args: Vec<Val>,
    blocks: Vec<std::slice::Iter<'m, Statement>>,
    /// The caller's expression for the result.
    into: Option<Handle<Expression>>,
}

impl<'m> Frame<'m> {
    fn val(&self, h: Handle<Expression>) -> &Val {
        self.vals
            .get(h.index())
            .and_then(Option::as_ref)
            .unwrap_or(&UNKNOWN)
    }
}

impl<'m> Context<'m> {
    fn new(module: &'m Module) -> Self {
        let mut depths = Vec::with_capacity(module.types.len());
        for (_, ty) in module.types.iter() {
            let of = |h: Handle<Type>| depths.get(h.index()).copied().unwrap_or(u32::MAX);
            let depth = match &ty.inner {
                TypeInner::Scalar(_) => 0,
                TypeInner::Vector { .. } => 1,
                TypeInner::Matrix { .. } => 2,
                TypeInner::Array { base, .. } => of(*base).saturating_add(1),
                TypeInner::Struct { members, .. } => members
                    .iter()
                    .map(|m| of(m.ty))
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1),
                _ => u32::MAX,
            };
            depths.push(depth);
        }
        let mut context = Self {
            module,
            globals: Vec::with_capacity(module.global_expressions.len()),
            depths,
            stores: Vec::with_capacity(module.functions.len()),
            size: module_size(module),
            budget: Cell::new(0),
        };
        for (h, e) in module.global_expressions.iter() {
            let globals = &context.globals;
            let get = |x: Handle<Expression>| globals.get(x.index()).unwrap_or(&UNKNOWN);
            let v = if x_before(h, e) {
                context.pure(e, &get)
            } else {
                Val::Unknown
            };
            context.globals.push(v);
        }
        // The sets together hold at most the module's size: past it, a function's is unknown
        // and a call that doesn't run makes every global unknown.
        let mut left = context.size;
        for (h, f) in module.functions.iter() {
            let stores = context.stores_of(h, f, &mut left);
            context.stores.push(stores);
        }
        context
    }

    /// The globals `function` and its callees may store to, each entry taken from `left`
    /// before it is added.
    fn stores_of(
        &self,
        id: Handle<Function>,
        function: &Function,
        left: &mut u64,
    ) -> Option<BTreeSet<Handle<GlobalVariable>>> {
        let mut set = BTreeSet::new();
        let take = |left: &mut u64| {
            *left = left.checked_sub(1)?;
            Some(())
        };
        let mut blocks = vec![&function.body];
        while let Some(block) = blocks.pop() {
            for statement in block {
                match statement {
                    Statement::Store { pointer, .. } | Statement::Atomic { pointer, .. } => {
                        match base_variable(function, *pointer, &mut || take(left))? {
                            Expression::GlobalVariable(g) => {
                                take(left)?;
                                set.insert(*g);
                            }
                            // The caller makes what a pointer argument points to unknown.
                            Expression::LocalVariable(_) | Expression::FunctionArgument(_) => {}
                            _ => return None,
                        }
                    }
                    Statement::Call { function: f, .. } => {
                        if f.index() >= id.index() {
                            return None;
                        }
                        for &g in self.stores.get(f.index())?.as_ref()? {
                            take(left)?;
                            set.insert(g);
                        }
                    }
                    Statement::Block(b) => blocks.push(b),
                    Statement::If { accept, reject, .. } => blocks.extend([accept, reject]),
                    Statement::Switch { cases, .. } => blocks.extend(cases.iter().map(|c| &c.body)),
                    Statement::Loop {
                        body, continuing, ..
                    } => blocks.extend([body, continuing]),
                    _ => {}
                }
            }
        }
        Some(set)
    }

    fn depth(&self, ty: Handle<Type>) -> u32 {
        self.depths.get(ty.index()).copied().unwrap_or(u32::MAX)
    }

    /// The zero value of `ty`; unknown past [`MAX_DEPTH`].
    fn zero(&self, ty: Handle<Type>) -> Val {
        if self.depth(ty) > MAX_DEPTH {
            return Val::Unknown;
        }
        self.zero_of(&self.module.types[ty].inner)
    }

    fn zero_of(&self, inner: &TypeInner) -> Val {
        match inner {
            TypeInner::Scalar(s) => match (s.kind, s.width) {
                (ScalarKind::Float, 4) => Val::F(0.0),
                (ScalarKind::Sint, 4) => Val::I(0),
                (ScalarKind::Uint, 4) => Val::U(0),
                (ScalarKind::Bool, _) => Val::B(false),
                _ => Val::Unknown,
            },
            TypeInner::Vector { size, scalar } => {
                list(vec![
                    self.zero_of(&TypeInner::Scalar(*scalar));
                    *size as usize
                ])
            }
            TypeInner::Matrix {
                columns,
                rows,
                scalar,
            } => mat(vec![
                self.zero_of(&TypeInner::Vector {
                    size: *rows,
                    scalar: *scalar
                });
                *columns as usize
            ]),
            TypeInner::Array {
                base,
                size: ArraySize::Constant(n),
                ..
            } => Val::Array {
                len: n.get(),
                set: Rc::default(),
                rest: Rc::new(self.zero(*base)),
            },
            TypeInner::Struct { members, .. } => {
                list(members.iter().map(|m| self.zero(m.ty)).collect())
            }
            _ => Val::Unknown,
        }
    }

    /// `v` with its unknown containers along `path` made of unknown parts, in `inner`'s
    /// shape, and `x` stored at `path`; `false` when `path` doesn't fit the shape.
    fn store(&self, v: &mut Val, inner: &TypeInner, path: &[u32], x: Val) -> bool {
        let Some((&i, rest)) = path.split_first() else {
            *v = x;
            return true;
        };
        let child: TypeInner = match inner {
            TypeInner::Vector { scalar, .. } => TypeInner::Scalar(*scalar),
            TypeInner::Matrix { rows, scalar, .. } => TypeInner::Vector {
                size: *rows,
                scalar: *scalar,
            },
            TypeInner::Array { base, .. } => self.module.types[*base].inner.clone(),
            TypeInner::Struct { members, .. } => match members.get(i as usize) {
                Some(m) => self.module.types[m.ty].inner.clone(),
                None => return false,
            },
            _ => return false,
        };
        if *v == Val::Unknown {
            *v = match inner {
                TypeInner::Vector { size, .. } => list(vec![Val::Unknown; *size as usize]),
                TypeInner::Matrix { columns, .. } => mat(vec![Val::Unknown; *columns as usize]),
                TypeInner::Array {
                    size: ArraySize::Constant(n),
                    ..
                } => Val::Array {
                    len: n.get(),
                    set: Rc::default(),
                    rest: Rc::new(Val::Unknown),
                },
                TypeInner::Struct { members, .. } => list(vec![Val::Unknown; members.len()]),
                _ => return false,
            };
        }
        match v {
            Val::List(items) | Val::Mat(items) => {
                if i as usize >= items.len() || !self.copy(Rc::strong_count(items), items.len()) {
                    return false;
                }
                let item = &mut Rc::make_mut(items)[i as usize];
                self.store(item, &child, rest, x)
            }
            Val::Array { len, set, rest: r } if i < *len => {
                if !self.copy(Rc::strong_count(set), set.len()) {
                    return false;
                }
                let item = Rc::make_mut(set).entry(i).or_insert_with(|| (**r).clone());
                self.store(item, &child, rest, x)
            }
            _ => false,
        }
    }

    /// Charges a container of `len` items before it is copied for a write (`strong` holders
    /// share it); `false` past the budget.
    fn copy(&self, strong: usize, len: usize) -> bool {
        strong == 1 || self.charge(len).is_some()
    }

    /// Takes `n` units of work from this vertex's budget, before the work.
    fn charge(&self, n: usize) -> Option<()> {
        let n = u64::try_from(n).ok()?;
        let left = self.budget.get().checked_sub(n)?;
        self.budget.set(left);
        Some(())
    }

    fn root_type(&self, root: Root) -> Option<Handle<Type>> {
        match root {
            Root::Global(g) => Some(self.module.global_variables[g].ty),
            Root::Local(f, l) => {
                let function = match f {
                    Some(f) => &self.module.functions[f],
                    None => {
                        &(self.module.entry_points.iter())
                            .find(|e| e.stage == ShaderStage::Vertex)?
                            .function
                    }
                };
                Some(function.local_variables[l].ty)
            }
        }
    }

    /// A value made from known operands, whatever the function: the expressions that need no
    /// variable, argument or call.
    fn pure<'v>(&self, e: &Expression, get: &dyn Fn(Handle<Expression>) -> &'v Val) -> Val {
        match e {
            Expression::Literal(l) => match *l {
                Literal::F32(f) if f == 0.0 || f.is_normal() => Val::F(f),
                Literal::I32(i) => Val::I(i),
                Literal::U32(u) => Val::U(u),
                Literal::Bool(b) => Val::B(b),
                _ => Val::Unknown,
            },
            Expression::Constant(c) => {
                let init = self.module.constants[*c].init;
                self.globals
                    .get(init.index())
                    .cloned()
                    .unwrap_or(Val::Unknown)
            }
            Expression::ZeroValue(ty) => self.zero(*ty),
            Expression::Compose { ty, components } => {
                if self.depth(*ty) > MAX_DEPTH {
                    return Val::Unknown;
                }
                let parts = components.iter().map(|&c| get(c));
                match &self.module.types[*ty].inner {
                    // A vector composes from scalars and vectors.
                    TypeInner::Vector { size, .. } => {
                        let mut items = Vec::new();
                        for part in parts {
                            match part {
                                Val::List(v) => items.extend(v.iter().cloned()),
                                Val::Unknown => return Val::Unknown,
                                scalar => items.push(scalar.clone()),
                            }
                        }
                        if items.len() == *size as usize {
                            list(items)
                        } else {
                            Val::Unknown
                        }
                    }
                    TypeInner::Matrix { columns, .. } => {
                        let cols: Vec<Val> = parts.cloned().collect();
                        if cols.len() == *columns as usize
                            && cols.iter().all(|c| matches!(c, Val::List(_)))
                        {
                            mat(cols)
                        } else {
                            Val::Unknown
                        }
                    }
                    TypeInner::Array {
                        size: ArraySize::Constant(n),
                        ..
                    } if n.get() as usize == components.len() => Val::Array {
                        len: n.get(),
                        set: Rc::new(
                            parts
                                .cloned()
                                .enumerate()
                                .map(|(i, v)| (i as u32, v))
                                .collect(),
                        ),
                        rest: Rc::new(Val::Unknown),
                    },
                    TypeInner::Struct { .. } => list(parts.cloned().collect()),
                    _ => Val::Unknown,
                }
            }
            Expression::Splat { size, value } => match get(*value) {
                Val::Unknown => Val::Unknown,
                v => list(vec![v.clone(); *size as usize]),
            },
            Expression::Swizzle {
                size,
                vector,
                pattern,
            } => match get(*vector) {
                Val::List(items) => list(
                    pattern[..*size as usize]
                        .iter()
                        .map(|&c| items.get(c as usize).cloned().unwrap_or(Val::Unknown))
                        .collect(),
                ),
                _ => Val::Unknown,
            },
            Expression::AccessIndex { base, index } => element(get(*base), Some(*index)),
            Expression::Access { base, index } => element(get(*base), get(*index).index()),
            Expression::Unary { op, expr } => map(get(*expr), &|v| unary(*op, v)),
            Expression::Binary { op, left, right } => binary(*op, get(*left), get(*right)),
            Expression::Select {
                condition,
                accept,
                reject,
            } => select(get(*condition), get(*accept), get(*reject)),
            Expression::Relational { fun, argument } => match (fun, get(*argument)) {
                (RelationalFunction::All | RelationalFunction::Any, Val::List(items)) => {
                    let bools: Option<Vec<bool>> = items
                        .iter()
                        .map(|v| match v {
                            Val::B(b) => Some(*b),
                            _ => None,
                        })
                        .collect();
                    match bools {
                        Some(b) if *fun == RelationalFunction::All => Val::B(b.iter().all(|&x| x)),
                        Some(b) => Val::B(b.iter().any(|&x| x)),
                        None => Val::Unknown,
                    }
                }
                _ => Val::Unknown,
            },
            Expression::Math {
                fun,
                arg,
                arg1,
                arg2,
                ..
            } => {
                let a = get(*arg);
                let b = arg1.map(get);
                let c = arg2.map(get);
                math(*fun, a, b, c)
            }
            Expression::As {
                expr,
                kind,
                convert: Some(4),
            } => map(get(*expr), &|v| convert(v, *kind)),
            _ => Val::Unknown,
        }
    }

    /// The position the vertex stage gives vertex `index`, when known exactly.
    fn position(&self, entry: &'m Function, index: u32) -> Option<[f32; 4]> {
        let module = self.module;
        self.budget.set(self.size);
        let mut state: HashMap<Root, Val> = HashMap::new();
        let mut ran = vec![false; module.functions.len()];
        let args = entry
            .arguments
            .iter()
            .map(|a| {
                let n = match a.binding {
                    Some(Binding::BuiltIn(BuiltIn::VertexIndex)) => index,
                    Some(Binding::BuiltIn(BuiltIn::InstanceIndex)) => 0,
                    _ => return Val::Unknown,
                };
                match &module.types[a.ty].inner {
                    TypeInner::Scalar(s) if s.kind == ScalarKind::Uint => Val::U(n),
                    TypeInner::Scalar(s) if s.kind == ScalarKind::Sint => {
                        i32::try_from(n).map_or(Val::Unknown, Val::I)
                    }
                    _ => Val::Unknown,
                }
            })
            .collect();
        let mut stack = vec![self.frame(None, entry, args, None, &mut state)];
        let result = loop {
            let frame = stack.last_mut()?;
            let Some(statement) = frame.blocks.last_mut().map(Iterator::next) else {
                // The end of a function without `return`.
                let done = stack.pop()?;
                if stack.is_empty() {
                    break Val::Unknown;
                }
                if let (Some(into), Some(caller)) = (done.into, stack.last_mut()) {
                    caller.vals[into.index()] = Some(Val::Unknown);
                }
                continue;
            };
            let Some(statement) = statement else {
                frame.blocks.pop();
                continue;
            };
            self.charge(1)?;
            match statement {
                Statement::Emit(range) => {
                    for h in range.clone() {
                        self.charge(1)?;
                        let v = self.compute(frame, &state, h);
                        frame.vals[h.index()] = Some(v);
                    }
                }
                Statement::Block(b) => frame.blocks.push(b.iter()),
                Statement::If {
                    condition,
                    accept,
                    reject,
                } => match frame.val(*condition) {
                    Val::B(true) => frame.blocks.push(accept.iter()),
                    Val::B(false) => frame.blocks.push(reject.iter()),
                    _ => {
                        let blocks = vec![accept.iter().as_slice(), reject.iter().as_slice()];
                        if self.havoc(frame, blocks, &mut state)? {
                            unknown_return(&mut stack, &mut state, self)?;
                        }
                    }
                },
                Statement::Switch { cases, .. } => {
                    let blocks = cases.iter().map(|c| c.body.iter().as_slice()).collect();
                    if self.havoc(frame, blocks, &mut state)? {
                        unknown_return(&mut stack, &mut state, self)?;
                    }
                }
                Statement::Loop {
                    body, continuing, ..
                } => {
                    let blocks = vec![body.iter().as_slice(), continuing.iter().as_slice()];
                    if self.havoc(frame, blocks, &mut state)? {
                        unknown_return(&mut stack, &mut state, self)?;
                    }
                }
                Statement::Return { value } => {
                    let v = value.map_or(Val::Unknown, |h| frame.val(h).clone());
                    let done = stack.pop()?;
                    match stack.last_mut() {
                        None => break v,
                        Some(caller) => {
                            if let Some(into) = done.into {
                                caller.vals[into.index()] = Some(v);
                            }
                        }
                    }
                }
                Statement::Store { pointer, value } => {
                    let v = frame.val(*value).clone();
                    match frame.val(*pointer).clone() {
                        Val::Ptr(root, path) => {
                            let ty = self.root_type(root)?;
                            // In place: the variable's value is shared with nothing, so a
                            // store copies nothing.
                            let initial = self.read(&state, root);
                            let whole = state.entry(root).or_insert(initial);
                            let ok = self.depth(ty) <= MAX_DEPTH
                                && self.store(whole, &module.types[ty].inner, &path, v);
                            if !ok {
                                *whole = Val::Unknown;
                            }
                        }
                        Val::Somewhere(root) => {
                            state.insert(root, Val::Unknown);
                        }
                        _ => return None,
                    }
                }
                Statement::Call {
                    function,
                    arguments,
                    result,
                } => {
                    let args: Vec<Val> = arguments.iter().map(|&a| frame.val(a).clone()).collect();
                    if std::mem::replace(&mut ran[function.index()], true) {
                        // Run once per vertex: a later call makes what it may store unknown.
                        self.havoc_call(*function, &args, &mut state)?;
                        if let Some(r) = result {
                            frame.vals[r.index()] = Some(Val::Unknown);
                        }
                    } else {
                        let callee = &module.functions[*function];
                        let f = self.frame(Some(*function), callee, args, *result, &mut state);
                        stack.push(f);
                    }
                }
                Statement::Atomic {
                    pointer, result, ..
                } => {
                    if let Val::Ptr(root, _) | Val::Somewhere(root) = frame.val(*pointer) {
                        state.insert(*root, Val::Unknown);
                    }
                    if let Some(r) = result {
                        frame.vals[r.index()] = Some(Val::Unknown);
                    }
                }
                Statement::WorkGroupUniformLoad { result, .. }
                | Statement::SubgroupBallot { result, .. }
                | Statement::SubgroupGather { result, .. }
                | Statement::SubgroupCollectiveOperation { result, .. } => {
                    frame.vals[result.index()] = Some(Val::Unknown);
                }
                Statement::ImageStore { .. }
                | Statement::ImageAtomic { .. }
                | Statement::ControlBarrier(_)
                | Statement::MemoryBarrier(_) => {}
                // `break` and `continue` only run inside loops and switches, which aren't run.
                _ => return None,
            }
        };
        let value = match &entry.result {
            Some(r) if matches!(r.binding, Some(Binding::BuiltIn(BuiltIn::Position { .. }))) => {
                result
            }
            Some(r) => match &module.types[r.ty].inner {
                TypeInner::Struct { members, .. } => {
                    let at = members.iter().position(|m| {
                        matches!(m.binding, Some(Binding::BuiltIn(BuiltIn::Position { .. })))
                    })?;
                    result.element(u32::try_from(at).ok()?)
                }
                _ => return None,
            },
            None => return None,
        };
        let p = value.scalars()?;
        p.try_into().ok()
    }

    /// A new frame for `function`: its locals at their initial values in `state`, and the
    /// expressions no `Emit` covers evaluated.
    fn frame(
        &self,
        id: Option<Handle<Function>>,
        function: &'m Function,
        args: Vec<Val>,
        into: Option<Handle<Expression>>,
        state: &mut HashMap<Root, Val>,
    ) -> Frame<'m> {
        let mut emitted = vec![false; function.expressions.len()];
        let mut blocks = vec![&function.body];
        while let Some(block) = blocks.pop() {
            for statement in block {
                match statement {
                    Statement::Emit(range) => {
                        for h in range.clone() {
                            emitted[h.index()] = true;
                        }
                    }
                    Statement::Block(b) => blocks.push(b),
                    Statement::If { accept, reject, .. } => blocks.extend([accept, reject]),
                    Statement::Switch { cases, .. } => blocks.extend(cases.iter().map(|c| &c.body)),
                    Statement::Loop {
                        body, continuing, ..
                    } => blocks.extend([body, continuing]),
                    _ => {}
                }
            }
        }
        let mut frame = Frame {
            id,
            function,
            vals: vec![None; function.expressions.len()],
            args,
            blocks: vec![function.body.iter()],
            into,
        };
        for (h, e) in function.expressions.iter() {
            if emitted[h.index()] || matches!(e, Expression::Load { .. }) {
                continue;
            }
            let v = self.compute(&frame, state, h);
            frame.vals[h.index()] = Some(v);
        }
        for (h, local) in function.local_variables.iter() {
            let v = local
                .init
                .map_or(Val::Unknown, |init| frame.val(init).clone());
            state.insert(Root::Local(id, h), v);
        }
        frame
    }

    /// The value of a variable: as stored, or its initial one.
    fn read(&self, state: &HashMap<Root, Val>, root: Root) -> Val {
        if let Some(v) = state.get(&root) {
            return v.clone();
        }
        match root {
            Root::Global(g) => {
                let global = &self.module.global_variables[g];
                match (global.space, global.init) {
                    (AddressSpace::Private, Some(init)) => self
                        .globals
                        .get(init.index())
                        .cloned()
                        .unwrap_or(Val::Unknown),
                    _ => Val::Unknown,
                }
            }
            Root::Local(..) => Val::Unknown,
        }
    }

    fn compute(&self, frame: &Frame<'m>, state: &HashMap<Root, Val>, h: Handle<Expression>) -> Val {
        let e = &frame.function.expressions[h];
        match e {
            Expression::FunctionArgument(i) => {
                frame.args.get(*i as usize).cloned().unwrap_or(Val::Unknown)
            }
            Expression::GlobalVariable(g) => Val::Ptr(Root::Global(*g), Vec::new()),
            Expression::LocalVariable(l) => Val::Ptr(Root::Local(frame.id, *l), Vec::new()),
            Expression::Load { pointer } => match frame.val(*pointer) {
                Val::Ptr(root, path) => {
                    let global_space = match root {
                        Root::Global(g) => self.module.global_variables[*g].space,
                        Root::Local(..) => AddressSpace::Function,
                    };
                    match global_space {
                        AddressSpace::Private | AddressSpace::Function => {
                            self.read(state, *root).at(path)
                        }
                        _ => Val::Unknown,
                    }
                }
                _ => Val::Unknown,
            },
            Expression::Access { base, index } => match frame.val(*base) {
                Val::Ptr(root, path) => match frame.val(*index).index() {
                    Some(i) => pointer(*root, path, i),
                    None => Val::Somewhere(*root),
                },
                Val::Somewhere(root) => Val::Somewhere(*root),
                _ => self.pure(e, &|x| frame.val(x)),
            },
            Expression::AccessIndex { base, index } => match frame.val(*base) {
                Val::Ptr(root, path) => pointer(*root, path, *index),
                Val::Somewhere(root) => Val::Somewhere(*root),
                _ => self.pure(e, &|x| frame.val(x)),
            },
            Expression::CallResult(_) => frame.val(h).clone(),
            _ => self.pure(e, &|x| frame.val(x)),
        }
    }

    /// What `blocks` may store, made unknown: they don't run. Whether one may return; `None`
    /// when one can discard, or stores through a pointer that can't be followed.
    fn havoc(
        &self,
        frame: &Frame<'m>,
        blocks: Vec<&'m [Statement]>,
        state: &mut HashMap<Root, Val>,
    ) -> Option<bool> {
        let mut returns = false;
        let mut stack = blocks;
        while let Some(block) = stack.pop() {
            for statement in block {
                self.charge(1)?;
                match statement {
                    Statement::Return { .. } => returns = true,
                    Statement::Kill => return None,
                    Statement::Store { pointer, .. } | Statement::Atomic { pointer, .. } => {
                        let root = self.root_of(frame, *pointer)?;
                        state.insert(root, Val::Unknown);
                    }
                    Statement::Call {
                        function,
                        arguments,
                        ..
                    } => {
                        let args: Vec<Val> = arguments
                            .iter()
                            .map(|&a| match self.root_of(frame, a) {
                                Some(root) => Val::Ptr(root, Vec::new()),
                                None => Val::Unknown,
                            })
                            .collect();
                        self.havoc_call(*function, &args, state)?;
                    }
                    Statement::Block(b) => stack.push(b.iter().as_slice()),
                    Statement::If { accept, reject, .. } => {
                        stack.extend([accept.iter().as_slice(), reject.iter().as_slice()])
                    }
                    Statement::Switch { cases, .. } => {
                        stack.extend(cases.iter().map(|c| c.body.iter().as_slice()))
                    }
                    Statement::Loop {
                        body, continuing, ..
                    } => stack.extend([body.iter().as_slice(), continuing.iter().as_slice()]),
                    Statement::RayQuery { .. }
                    | Statement::RayPipelineFunction(_)
                    | Statement::CooperativeStore { .. } => return None,
                    _ => {}
                }
            }
        }
        Some(returns)
    }

    /// A call that doesn't run: the globals it may store to and what its pointer arguments
    /// point to become unknown.
    fn havoc_call(
        &self,
        function: Handle<Function>,
        args: &[Val],
        state: &mut HashMap<Root, Val>,
    ) -> Option<()> {
        match self.stores.get(function.index()).and_then(Option::as_ref) {
            Some(globals) => {
                self.charge(globals.len())?;
                for &g in globals {
                    state.insert(Root::Global(g), Val::Unknown);
                }
            }
            None => {
                self.charge(self.module.global_variables.len())?;
                for (g, _) in self.module.global_variables.iter() {
                    state.insert(Root::Global(g), Val::Unknown);
                }
            }
        }
        for arg in args {
            if let Val::Ptr(root, _) | Val::Somewhere(root) = arg {
                state.insert(*root, Val::Unknown);
            }
        }
        Some(())
    }

    /// The variable a pointer expression starts from.
    fn root_of(&self, frame: &Frame<'m>, pointer: Handle<Expression>) -> Option<Root> {
        match base_variable(frame.function, pointer, &mut || self.charge(1))? {
            Expression::GlobalVariable(g) => Some(Root::Global(*g)),
            Expression::LocalVariable(l) => Some(Root::Local(frame.id, *l)),
            Expression::FunctionArgument(i) => match frame.args.get(*i as usize)? {
                Val::Ptr(root, _) | Val::Somewhere(root) => Some(*root),
                _ => None,
            },
            _ => None,
        }
    }
}

/// The top frame after a statement that may have returned: the rest of its body may run or
/// not, so what it may store becomes unknown, and it returns an unknown value.
fn unknown_return<'m>(
    stack: &mut Vec<Frame<'m>>,
    state: &mut HashMap<Root, Val>,
    context: &Context<'m>,
) -> Option<()> {
    let frame = stack.last()?;
    let rest = frame.blocks.iter().map(|b| b.as_slice()).collect();
    context.havoc(frame, rest, state)?;
    let done = stack.pop()?;
    if let (Some(into), Some(caller)) = (done.into, stack.last_mut()) {
        caller.vals[into.index()] = Some(Val::Unknown);
    }
    Some(())
}

/// Whether a global expression reads only earlier ones, as naga orders them.
fn x_before(h: Handle<Expression>, e: &Expression) -> bool {
    let before = |x: &Handle<Expression>| x.index() < h.index();
    match e {
        Expression::Compose { components, .. } => components.iter().all(before),
        Expression::Splat { value, .. } => before(value),
        Expression::Swizzle { vector, .. } => before(vector),
        Expression::AccessIndex { base, .. } => before(base),
        Expression::Access { base, index } => before(base) && before(index),
        Expression::Unary { expr, .. } | Expression::As { expr, .. } => before(expr),
        Expression::Binary { left, right, .. } => before(left) && before(right),
        Expression::Select {
            condition,
            accept,
            reject,
        } => before(condition) && before(accept) && before(reject),
        Expression::Relational { argument, .. } => before(argument),
        Expression::Math {
            arg,
            arg1,
            arg2,
            arg3,
            ..
        } => {
            before(arg)
                && [arg1, arg2, arg3]
                    .iter()
                    .all(|a| a.as_ref().is_none_or(before))
        }
        _ => true,
    }
}

/// The expression at the start of a pointer's chain of accesses: a variable or an argument.
/// Each step is taken from `steps` first.
fn base_variable<'f>(
    function: &'f Function,
    mut pointer: Handle<Expression>,
    steps: &mut dyn FnMut() -> Option<()>,
) -> Option<&'f Expression> {
    // Each base is an earlier expression: the walk ends.
    loop {
        steps()?;
        let e = function.expressions.try_get(pointer).ok()?;
        match e {
            Expression::Access { base, .. } | Expression::AccessIndex { base, .. }
                if base.index() < pointer.index() =>
            {
                pointer = *base
            }
            Expression::Access { .. } | Expression::AccessIndex { .. } => return None,
            _ => return Some(e),
        }
    }
}

/// A pointer one access further than `path`. Past [`MAX_DEPTH`] no value is followed, so the
/// pointer is to somewhere in `root`, and the path stays short.
fn pointer(root: Root, path: &[u32], index: u32) -> Val {
    if path.len() >= MAX_DEPTH as usize {
        return Val::Somewhere(root);
    }
    Val::Ptr(root, [path, &[index]].concat())
}

fn element(base: &Val, index: Option<u32>) -> Val {
    match index {
        Some(i) => base.element(i),
        None => Val::Unknown,
    }
}

/// `f` on a scalar, or on each component of a vector or matrix.
fn map(v: &Val, f: &dyn Fn(&Val) -> Val) -> Val {
    match v {
        Val::List(items) => list(items.iter().map(|x| map(x, f)).collect()),
        Val::Mat(cols) => mat(cols.iter().map(|x| map(x, f)).collect()),
        Val::Unknown => Val::Unknown,
        scalar => f(scalar),
    }
}

/// `f` on two values' matching components, a scalar going with each component of the other.
fn zip(a: &Val, b: &Val, f: &dyn Fn(&Val, &Val) -> Val) -> Val {
    match (a, b) {
        (Val::Unknown, _) | (_, Val::Unknown) => Val::Unknown,
        (Val::List(x), Val::List(y)) if x.len() == y.len() => {
            list(x.iter().zip(y.iter()).map(|(p, q)| zip(p, q, f)).collect())
        }
        (Val::Mat(x), Val::Mat(y)) if x.len() == y.len() => {
            mat(x.iter().zip(y.iter()).map(|(p, q)| zip(p, q, f)).collect())
        }
        (Val::List(_) | Val::Mat(_), Val::List(_) | Val::Mat(_)) => Val::Unknown,
        (Val::List(x), s) => list(x.iter().map(|p| zip(p, s, f)).collect()),
        (Val::Mat(x), s) => mat(x.iter().map(|p| zip(p, s, f)).collect()),
        (s, Val::List(y)) => list(y.iter().map(|q| zip(s, q, f)).collect()),
        (s, Val::Mat(y)) => mat(y.iter().map(|q| zip(s, q, f)).collect()),
        (p, q) => f(p, q),
    }
}

fn float(v: Option<f32>) -> Val {
    v.map_or(Val::Unknown, Val::F)
}

fn unary(op: UnaryOperator, v: &Val) -> Val {
    match (op, v) {
        (UnaryOperator::Negate, Val::F(f)) => Val::F(-f),
        (UnaryOperator::Negate, Val::I(i)) => Val::I(i.wrapping_neg()),
        (UnaryOperator::LogicalNot, Val::B(b)) => Val::B(!b),
        (UnaryOperator::BitwiseNot, Val::I(i)) => Val::I(!i),
        (UnaryOperator::BitwiseNot, Val::U(u)) => Val::U(!u),
        _ => Val::Unknown,
    }
}

/// The columns of a matrix whose every component is a known float.
fn columns(v: &Val) -> Option<Vec<Vec<f32>>> {
    match v {
        Val::Mat(cols) => cols.iter().map(Val::scalars).collect(),
        _ => None,
    }
}

fn floats(v: Option<Vec<f32>>) -> Val {
    v.map_or(Val::Unknown, |v| list(v.into_iter().map(Val::F).collect()))
}

fn binary(op: BinaryOperator, a: &Val, b: &Val) -> Val {
    use BinaryOperator as B;
    if op == B::Multiply {
        match (a, b) {
            (Val::Mat(_), Val::List(_)) => {
                // Row r of the result: Σ_c m[c][r] × v[c].
                let (Some(m), Some(v)) = (columns(a), b.scalars()) else {
                    return Val::Unknown;
                };
                let rows = m.first().map_or(0, Vec::len);
                return floats(
                    (0..rows)
                        .map(|r| {
                            let row: Option<Vec<f32>> =
                                m.iter().map(|c| c.get(r).copied()).collect();
                            dot(&row?, &v)
                        })
                        .collect(),
                );
            }
            (Val::List(_), Val::Mat(_)) => {
                let (Some(v), Some(m)) = (a.scalars(), columns(b)) else {
                    return Val::Unknown;
                };
                return floats(m.iter().map(|c| dot(&v, c)).collect());
            }
            (Val::Mat(_), Val::Mat(_)) => {
                let (Some(x), Some(y)) = (columns(a), columns(b)) else {
                    return Val::Unknown;
                };
                let rows = x.first().map_or(0, Vec::len);
                let product: Option<Vec<Val>> = y
                    .iter()
                    .map(|col| {
                        let column: Option<Vec<f32>> = (0..rows)
                            .map(|r| {
                                let row: Option<Vec<f32>> =
                                    x.iter().map(|c| c.get(r).copied()).collect();
                                dot(&row?, col)
                            })
                            .collect();
                        Some(floats(Some(column?)))
                    })
                    .collect();
                return product.map_or(Val::Unknown, mat);
            }
            _ => {}
        }
    }
    zip(a, b, &|x, y| scalar_binary(op, x, y))
}

fn scalar_binary(op: BinaryOperator, a: &Val, b: &Val) -> Val {
    use BinaryOperator as B;
    match (a, b) {
        (Val::F(x), Val::F(y)) => {
            let (x, y) = (*x, *y);
            match op {
                B::Add => float(add(x, y)),
                B::Subtract => float(sub(x, y)),
                B::Multiply => float(mul(x, y)),
                B::Equal => Val::B(x == y),
                B::NotEqual => Val::B(x != y),
                B::Less => Val::B(x < y),
                B::LessEqual => Val::B(x <= y),
                B::Greater => Val::B(x > y),
                B::GreaterEqual => Val::B(x >= y),
                // Only sums, differences and products are checked for exactness.
                _ => Val::Unknown,
            }
        }
        (Val::I(x), Val::I(y)) => {
            let (x, y) = (*x, *y);
            match op {
                B::Add => Val::I(x.wrapping_add(y)),
                B::Subtract => Val::I(x.wrapping_sub(y)),
                B::Multiply => Val::I(x.wrapping_mul(y)),
                B::Divide if y != 0 && !(x == i32::MIN && y == -1) => Val::I(x / y),
                B::Modulo if x >= 0 && y > 0 => Val::I(x % y),
                B::Equal => Val::B(x == y),
                B::NotEqual => Val::B(x != y),
                B::Less => Val::B(x < y),
                B::LessEqual => Val::B(x <= y),
                B::Greater => Val::B(x > y),
                B::GreaterEqual => Val::B(x >= y),
                B::And => Val::I(x & y),
                B::InclusiveOr => Val::I(x | y),
                B::ExclusiveOr => Val::I(x ^ y),
                _ => Val::Unknown,
            }
        }
        (Val::U(x), Val::U(y)) => {
            let (x, y) = (*x, *y);
            match op {
                B::Add => Val::U(x.wrapping_add(y)),
                B::Subtract => Val::U(x.wrapping_sub(y)),
                B::Multiply => Val::U(x.wrapping_mul(y)),
                B::Divide if y != 0 => Val::U(x / y),
                B::Modulo if y != 0 => Val::U(x % y),
                B::Equal => Val::B(x == y),
                B::NotEqual => Val::B(x != y),
                B::Less => Val::B(x < y),
                B::LessEqual => Val::B(x <= y),
                B::Greater => Val::B(x > y),
                B::GreaterEqual => Val::B(x >= y),
                B::And => Val::U(x & y),
                B::InclusiveOr => Val::U(x | y),
                B::ExclusiveOr => Val::U(x ^ y),
                _ => Val::Unknown,
            }
        }
        (Val::B(x), Val::B(y)) => match op {
            B::Equal => Val::B(x == y),
            B::NotEqual => Val::B(x != y),
            B::LogicalAnd | B::And => Val::B(*x && *y),
            B::LogicalOr | B::InclusiveOr => Val::B(*x || *y),
            _ => Val::Unknown,
        },
        _ => Val::Unknown,
    }
}

fn select(condition: &Val, accept: &Val, reject: &Val) -> Val {
    match condition {
        Val::B(true) => accept.clone(),
        Val::B(false) => reject.clone(),
        Val::List(c) => match (accept, reject) {
            (Val::List(a), Val::List(r)) if a.len() == c.len() && r.len() == c.len() => list(
                c.iter()
                    .zip(a.iter().zip(r.iter()))
                    .map(|(c, (a, r))| select(c, a, r))
                    .collect(),
            ),
            _ => Val::Unknown,
        },
        // Equal scalars or vectors; anything larger isn't compared.
        _ if small(accept) && accept == reject => accept.clone(),
        _ => Val::Unknown,
    }
}

/// A scalar, or a vector of scalars.
fn small(v: &Val) -> bool {
    match v {
        Val::F(_) | Val::I(_) | Val::U(_) | Val::B(_) => true,
        Val::List(items) => {
            (items.iter()).all(|x| matches!(x, Val::F(_) | Val::I(_) | Val::U(_) | Val::B(_)))
        }
        _ => false,
    }
}

fn math(fun: MathFunction, a: &Val, b: Option<&Val>, c: Option<&Val>) -> Val {
    use MathFunction as M;
    let pair = |f: &dyn Fn(&Val, &Val) -> Val| b.map_or(Val::Unknown, |b| zip(a, b, f));
    match fun {
        M::Abs => map(a, &|v| match v {
            Val::F(f) => Val::F(f.abs()),
            Val::I(i) if *i != i32::MIN => Val::I(i.abs()),
            Val::U(u) => Val::U(*u),
            _ => Val::Unknown,
        }),
        M::Floor | M::Ceil | M::Trunc => map(a, &|v| match v {
            Val::F(f) => {
                let r = match fun {
                    M::Floor => f.floor(),
                    M::Ceil => f.ceil(),
                    _ => f.trunc(),
                };
                float(Exact::of(r).and_then(Exact::to_f32))
            }
            _ => Val::Unknown,
        }),
        M::Sign => map(a, &|v| match v {
            Val::F(f) if *f == 0.0 => Val::F(0.0),
            Val::F(f) => Val::F(f.signum()),
            Val::I(i) => Val::I(i.signum()),
            _ => Val::Unknown,
        }),
        M::Min | M::Max => pair(&|x, y| {
            let less = match (x, y) {
                (Val::F(p), Val::F(q)) => p < q,
                (Val::I(p), Val::I(q)) => p < q,
                (Val::U(p), Val::U(q)) => p < q,
                _ => return Val::Unknown,
            };
            if less == (fun == M::Min) {
                x.clone()
            } else {
                y.clone()
            }
        }),
        M::Clamp => match (b, c) {
            (Some(low), Some(high)) => {
                let raised = math(M::Max, a, Some(low), None);
                math(M::Min, &raised, Some(high), None)
            }
            _ => Val::Unknown,
        },
        M::Fma => match (b, c) {
            (Some(b), Some(c)) => zip(
                &zip(a, b, &|x, y| match (x, y) {
                    (Val::F(p), Val::F(q)) => {
                        float(Exact::of(*p).and_then(|p| p.mul(Exact::of(*q)?)?.to_f32()))
                    }
                    _ => Val::Unknown,
                }),
                c,
                &|x, y| scalar_binary(BinaryOperator::Add, x, y),
            ),
            _ => Val::Unknown,
        },
        M::Dot => match (a.scalars(), b.and_then(Val::scalars)) {
            (Some(x), Some(y)) => float(dot(&x, &y)),
            _ => Val::Unknown,
        },
        _ => Val::Unknown,
    }
}

fn convert(v: &Val, kind: ScalarKind) -> Val {
    match (v, kind) {
        (Val::F(f), ScalarKind::Float) => Val::F(*f),
        (Val::I(i), ScalarKind::Float) => float(exact_int(i64::from(*i))),
        (Val::U(u), ScalarKind::Float) => float(exact_int(i64::from(*u))),
        (Val::B(b), ScalarKind::Float) => Val::F(f32::from(u8::from(*b))),
        (Val::F(f), ScalarKind::Sint) if f.trunc().abs() < 2f32.powi(31) => Val::I(*f as i32),
        (Val::F(f), ScalarKind::Uint) if *f > -1.0 && *f < 2f32.powi(32) => Val::U(*f as u32),
        (Val::I(i), ScalarKind::Sint) => Val::I(*i),
        (Val::U(u), ScalarKind::Uint) => Val::U(*u),
        (Val::I(i), ScalarKind::Uint) => Val::U(*i as u32),
        (Val::U(u), ScalarKind::Sint) => Val::I(*u as i32),
        (Val::B(b), ScalarKind::Sint) => Val::I(i32::from(*b)),
        (Val::B(b), ScalarKind::Uint) => Val::U(u32::from(*b)),
        (Val::B(b), ScalarKind::Bool) => Val::B(*b),
        (Val::I(i), ScalarKind::Bool) => Val::B(*i != 0),
        (Val::U(u), ScalarKind::Bool) => Val::B(*u != 0),
        (Val::F(f), ScalarKind::Bool) => Val::B(*f != 0.0),
        _ => Val::Unknown,
    }
}

/// An integer as the f32 holding it exactly.
fn exact_int(i: i64) -> Option<f32> {
    Exact {
        m: i128::from(i),
        e: 0,
    }
    .to_f32()
}

/// One program of the frame, as the frame-start swap reads it: the buffers whose other side
/// it writes (`written`) or exposes (`flip`), those it replaces whole (`rewrites`), and
/// whether some frames skip it (`setup` runs on the frames after a load or a resize;
/// `PackPost::sizes_differ` skips a program while its buffers' sizes differ).
#[derive(Clone, Copy, Debug)]
pub struct Touch {
    pub written: u16,
    pub flip: u16,
    pub rewrites: u16,
    pub sometimes: bool,
}

/// The buffers of `candidates` a frame may start on the side the last one ended on, instead
/// of copying it back (#6858): in every frame, the first program to write or expose their other
/// side replaces it whole. That side holds the picture of one write earlier, where the copy
/// would have left the current one, so it may differ until then.
pub fn swaps(touches: impl IntoIterator<Item = Touch>, candidates: u16) -> u16 {
    // Buffers still waiting for the program that settles them.
    let mut open = candidates;
    let mut proven = 0u16;
    for t in touches {
        let touched = open & (t.written | t.flip);
        // A program that writes part of the other side or flips it unwritten ends the swap.
        open &= !(touched & !t.rewrites);
        if !t.sometimes {
            proven |= touched & t.rewrites;
            open &= !touched;
        }
    }
    // Never touched, or touched only by programs that replace them whole.
    proven | open
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod tests;
