//! Special item models drawn by mods' renderers on the client:
//! `native_register_special_model_renderer` (NeoForge's `RegisterSpecialModelRendererEvent`) is
//! asked once at start, before resources load, and its types are what a `minecraft:special` model
//! may name besides vanilla's, or in place of one; a model resolves its type to the renderer's
//! slot when it decodes. When an item of such a model is resolved for drawing, on the main thread,
//! the type's renderer answers the resource models to draw (`ItemEnv::special_model_parts`),
//! checked here, then prepared by the item renderer, which bakes them once. A renderer registered
//! with `SpecialModelPolicy::Stack` has its answer kept by the stack for each model node for as
//! long as that stack is drawn unchanged, so it is asked once across frames, every display
//! context draws the same answer, and a draw looks nothing up by name. One registered with
//! `Frame` is asked for each frame, display context and game time an item is drawn with, and
//! its answers are kept only for that frame of that view.
use super::native_entity_renderers::Resolved;
use mod_api::{
    ClientHost, ItemDisplayContext, MAX_NESTED_STACK_PARTS, MAX_SPECIAL_MODEL_PARTS,
    SpecialModelContext, SpecialModelPart, SpecialModelPolicy, SpecialModelRenderer,
    SpecialPartContent,
};
use serde_json::Value;
use std::{
    hash::{Hash, Hasher},
    sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock, PoisonError, atomic::Ordering},
};
use sunrise_assets::{
    item_model::{PreparedParts, SpecialContent, SpecialModelSlot, SpecialPart, StackView},
    math::Mat4,
};
use sunrise_client::fast_hash::{FxHashMap, FxHasher};
use sunrise_core::nbt::Tag;

/// The registrations, by slot ([`SpecialModelSlot`]): each type and its renderer, kept once at
/// start.
static REGISTERED: OnceLock<
    Vec<(
        String,
        SpecialModelPolicy,
        Resolved<dyn SpecialModelRenderer>,
    )>,
> = OnceLock::new();

/// The prepared answers of the stacks drawn since the last frame began.
static ANSWERS: LazyLock<Mutex<Answers>> = LazyLock::new(Mutex::default);

fn answers() -> MutexGuard<'static, Answers> {
    ANSWERS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keeps the mods' registrations in slot order, once per process (false when they already were).
pub(super) fn register(
    renderers: Vec<(String, SpecialModelPolicy, Arc<dyn SpecialModelRenderer>)>,
) -> bool {
    REGISTERED
        .set(
            renderers
                .into_iter()
                .map(|(kind, policy, renderer)| (kind, policy, Resolved::new(renderer)))
                .collect(),
        )
        .is_ok()
}

/// On every resource reload: the renderers that failed are asked again, and no answer is kept
/// (the model nodes and baked models they belonged to are gone).
pub(in crate::game) fn reload() {
    for (_, _, resolved) in REGISTERED.get().into_iter().flatten() {
        resolved.failed.store(false, Ordering::Relaxed);
    }
    let mut answers = answers();
    answers.kept.clear();
    answers.refused.clear();
    answers.per_frame.clear();
}

/// Once per frame of the view `view` (its game's address), before it is drawn: the answers of
/// stacks no view's last frame drew are dropped, so what is kept is bounded by what is on screen.
pub(in crate::game) fn begin_frame(view: usize) {
    answers().begin_frame(view);
}

/// What the renderer of `slot` draws for `stack` in the model node of `config`, drawn as
/// `context` says: under `Stack`, the kept answer when this node drew an equal stack last frame or
/// this one, else the renderer's, checked, prepared by `prepare` and kept; under `Frame`, the
/// answer of this frame for the same stack, display context and time, else the renderer's. `None`
/// when no mod registered it, when it failed since resources last loaded, or when it fails now.
pub(in crate::game) fn parts(
    host: &dyn ClientHost,
    slot: SpecialModelSlot,
    config: &Arc<Tag>,
    stack: &StackView,
    context: &SpecialModelContext,
    prepare: impl FnOnce(&[SpecialPart]) -> PreparedParts,
) -> Option<PreparedParts> {
    let (kind, policy, resolved) = REGISTERED.get()?.get(slot.0 as usize)?;
    if resolved.failed.load(Ordering::Relaxed) {
        return None;
    }
    let (frame, context) = match policy {
        SpecialModelPolicy::Frame => (Some(FrameTiming::of(context)), context),
        _ => (None, &SpecialModelContext::STACK),
    };
    answer(&ANSWERS, Arc::as_ptr(config) as usize, stack, frame, || {
        // A stack the API can't take is refused alone: the renderer isn't at fault.
        use super::api_values::TryIntoApi;
        let api_stack = stack
            .try_into_api()
            .map_err(|e| warn_unreadable_once(kind, &stack.item, || format!("{e:?}")))
            .ok()?;
        let parts = guarded(kind, resolved, || {
            resolved.inner.extract(host, &api_stack, config, context)
        })?;
        Some(prepare(&parts))
    })
}

/// Says once per type and item that a stack of the item couldn't be given to the type's
/// renderer, so it draws the missing model.
fn warn_unreadable_once(kind: &str, item: &str, error: impl FnOnce() -> String) {
    static SEEN: Mutex<std::collections::BTreeMap<String, std::collections::BTreeSet<String>>> =
        Mutex::new(std::collections::BTreeMap::new());
    let mut seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner);
    if seen.get(kind).is_some_and(|items| items.contains(item)) {
        return;
    }
    seen.entry(kind.to_owned())
        .or_default()
        .insert(item.to_owned());
    drop(seen);
    let error = error();
    log::warn!(
        "Sunrise: a stack of {item} can't be read for the special model renderer of {kind} ({error}); it draws the missing model"
    );
}

/// What a `Frame` renderer's answer depends on besides the stack and the model node.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FrameTiming {
    display: ItemDisplayContext,
    game_time: Option<i64>,
    /// The bits of the partial tick, so equal ticks only are equal.
    partial_tick: u32,
}
impl FrameTiming {
    fn of(context: &SpecialModelContext) -> Self {
        Self {
            display: context.display,
            game_time: context.game_time,
            partial_tick: context.partial_tick.to_bits(),
        }
    }
}

/// The answer for `stack` in the model node `node`. With no `frame` (the `Stack` policy), the one
/// kept in `answers` for an equal stack, else what `ask` answers, kept. With one, the answer given
/// this frame of this view to an equal stack with that timing, else what `ask` answers, kept for
/// the frame, a refusal included. The lock isn't held while `ask` runs the mod's renderer.
fn answer(
    answers: &Mutex<Answers>,
    node: usize,
    stack: &StackView,
    frame: Option<FrameTiming>,
    ask: impl FnOnce() -> Option<PreparedParts>,
) -> Option<PreparedParts> {
    let lock = || answers.lock().unwrap_or_else(PoisonError::into_inner);
    let key = (node, stack_hash(stack));
    if let Some(timing) = frame {
        let (view, frame) = {
            let answers = lock();
            (answers.view, answers.frame)
        };
        let key = FrameKey {
            node,
            hash: key.1,
            view,
            frame,
            timing,
        };
        if let Some(kept) = lock().per_frame.get(&key).and_then(|answers| {
            answers
                .iter()
                .find_map(|(kept, prepared)| (kept == stack).then(|| prepared.clone()))
        }) {
            return kept;
        }
        let prepared = ask();
        lock()
            .per_frame
            .entry(key)
            .or_default()
            .push((stack.clone(), prepared.clone()));
        return prepared;
    }
    if let Some(kept) = lock().drawn(key, stack) {
        return Some(kept);
    }
    if lock()
        .refused
        .get(&key)
        .is_some_and(|stacks| stacks.contains(stack))
    {
        return None;
    }
    let Some(prepared) = ask() else {
        lock().refused.entry(key).or_default().push(stack.clone());
        return None;
    };
    lock().keep(key, stack, prepared.clone());
    Some(prepared)
}

/// The prepared answers kept, by model node (its `config`'s address, stable until resources
/// reload) and the stack's [`stack_hash`].
#[derive(Default)]
struct Answers {
    /// Counts the frames begun, by any view.
    frame: u64,
    /// The view whose frame began last: the one drawing.
    view: usize,
    /// The answers of `Frame` renderers, for the frame being drawn only.
    per_frame: FxHashMap<FrameKey, Vec<(StackView, Option<PreparedParts>)>>,
    /// Each view's last two frames: when its previous one began and its current one.
    views: FxHashMap<usize, [u64; 2]>,
    kept: FxHashMap<(usize, u64), Vec<Kept>>,
    refused: FxHashMap<(usize, u64), Vec<StackView>>,
}
/// Whose `Frame` answer it is: the model node, the stack's hash, the frame of the view drawing it
/// and what it was drawn with.
#[derive(PartialEq, Eq, Hash)]
struct FrameKey {
    node: usize,
    hash: u64,
    view: usize,
    frame: u64,
    timing: FrameTiming,
}
/// A view no frame of which began for this many frames (of all views) is gone.
const VIEW_GONE_AFTER: u64 = 256;
struct Kept {
    stack: StackView,
    prepared: PreparedParts,
    /// The frame (of any view) that last drew it.
    drawn: u64,
}
impl Answers {
    /// The answer kept for a stack equal to `stack`, now drawn in this frame.
    fn drawn(&mut self, key: (usize, u64), stack: &StackView) -> Option<PreparedParts> {
        let frame = self.frame;
        let kept = self
            .kept
            .get_mut(&key)?
            .iter_mut()
            .find(|k| k.stack == *stack)?;
        kept.drawn = frame;
        Some(kept.prepared.clone())
    }
    fn keep(&mut self, key: (usize, u64), stack: &StackView, prepared: PreparedParts) {
        self.kept.entry(key).or_default().push(Kept {
            stack: stack.clone(),
            prepared,
            drawn: self.frame,
        });
    }
    /// Starts the next frame of `view`, keeping only the answers drawn since the oldest of the
    /// views' previous frames began: what some view's last frame may have drawn.
    fn begin_frame(&mut self, view: usize) {
        self.refused.clear();
        self.per_frame.clear();
        self.view = view;
        self.frame += 1;
        let frame = self.frame;
        // A view's first frame keeps what was drawn before it began, as if its own.
        let starts = self.views.entry(view).or_insert([frame - 1; 2]);
        *starts = [starts[1], frame];
        self.views
            .retain(|_, [_, current]| frame - *current < VIEW_GONE_AFTER);
        let since = self.views.values().map(|[previous, _]| *previous).min();
        let since = since.unwrap_or(frame);
        self.kept.retain(|_, kept| {
            kept.retain(|k| k.drawn >= since);
            !kept.is_empty()
        });
    }
}

/// A hash of `stack` that equal stacks share, read in place.
fn stack_hash(stack: &StackView) -> u64 {
    let mut h = FxHasher::default();
    stack.item.hash(&mut h);
    stack.count.hash(&mut h);
    stack.bundle_selected.hash(&mut h);
    for (component, value) in &stack.patch {
        component.hash(&mut h);
        match value {
            Some(value) => value_hash(value).hash(&mut h),
            None => h.write_u8(0xFF),
        }
    }
    h.finish()
}
/// A hash of `value` that equal values share; an object's keys in any order.
fn value_hash(value: &Value) -> u64 {
    let mut h = FxHasher::default();
    std::mem::discriminant(value).hash(&mut h);
    match value {
        Value::Null => h.write_u8(0),
        Value::Bool(b) => b.hash(&mut h),
        Value::Number(n) => n.as_f64().map(f64::to_bits).hash(&mut h),
        Value::String(s) => s.hash(&mut h),
        Value::Array(items) => {
            for item in items {
                value_hash(item).hash(&mut h);
            }
        }
        Value::Object(fields) => fields
            .iter()
            .fold(0u64, |sum, (key, value)| {
                let mut field = FxHasher::default();
                key.hash(&mut field);
                value_hash(value).hash(&mut field);
                sum.wrapping_add(field.finish())
            })
            .hash(&mut h),
    }
    h.finish()
}

/// Runs `extract` for the renderer `resolved` of `kind` and checks its answer: `None`, with
/// `failed` set and logged once, when it returns an `Err`, panics or answers parts the resolver
/// refuses.
fn guarded(
    kind: &str,
    resolved: &Resolved<dyn SpecialModelRenderer>,
    extract: impl FnOnce() -> Result<Vec<SpecialModelPart>, String>,
) -> Option<Vec<SpecialPart>> {
    if resolved.failed.load(Ordering::Relaxed) {
        return None;
    }
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(extract))
        .unwrap_or_else(|_| Err("it panicked".into()))
        .and_then(|parts| checked(&parts));
    match answer {
        Ok(parts) => Some(parts),
        Err(error) => {
            if !resolved.failed.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "Sunrise: the special model renderer of {kind} failed ({error}); its models draw the missing model until resources reload"
                );
            }
            None
        }
    }
}

/// `parts` as the resolver takes them, or why they are refused: more than
/// [`MAX_SPECIAL_MODEL_PARTS`], more than [`MAX_NESTED_STACK_PARTS`] stacks, a stack the game
/// can't read, or a transform that isn't finite. Nothing is kept of a refusal. Cycles between
/// stacks are refused when the item is drawn, where the resolver expands them.
fn checked(parts: &[SpecialModelPart]) -> Result<Vec<SpecialPart>, String> {
    if parts.len() > MAX_SPECIAL_MODEL_PARTS {
        return Err(format!(
            "{} parts, more than {MAX_SPECIAL_MODEL_PARTS}",
            parts.len()
        ));
    }
    let stacks = parts
        .iter()
        .filter(|part| matches!(part.content, SpecialPartContent::Stack { .. }))
        .count();
    if stacks > MAX_NESTED_STACK_PARTS {
        return Err(format!(
            "{stacks} stack parts, more than {MAX_NESTED_STACK_PARTS}"
        ));
    }
    parts
        .iter()
        .map(|part| {
            let [a, b, c, d, e, f, g, h, i, j, k, l] = part.transform;
            let pose =
                Mat4::from_row_major([a, b, c, d, e, f, g, h, i, j, k, l, 0.0, 0.0, 0.0, 1.0]);
            let content = match &part.content {
                SpecialPartContent::Model(model) => SpecialContent::Model(model.clone()),
                SpecialPartContent::Stack { stack, display } => {
                    let stack = super::api_values::stack_value(stack)
                        .map_err(|e| format!("a stack part can't be read: {e:?}"))?;
                    let display = super::api_values::engine_display_context(*display)
                        .ok_or("a stack part's display context is unknown")?;
                    SpecialContent::Stack {
                        stack: sunrise_client::item_render::stack_view(&stack),
                        display,
                    }
                }
                _ => return Err("a part of a kind this game doesn't have".into()),
            };
            if !pose.is_finite() {
                let what = match &part.content {
                    SpecialPartContent::Model(model) => model,
                    _ => "a stack part",
                };
                return Err(format!("{what}'s transform isn't finite"));
            }
            Ok(SpecialPart {
                content,
                pose,
                tints: part.tints.iter().map(|&argb| argb as i32).collect(),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "native_special_renderers/cost_tests.rs"]
mod cost_tests;
#[cfg(test)]
#[path = "native_special_renderers/frame_tests.rs"]
pub(in crate::game) mod frame_tests;

/// How many answers are kept, for every model node.
#[cfg(test)]
fn kept_answers() -> usize {
    answers().kept.values().map(Vec::len).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Unused;
    impl SpecialModelRenderer for Unused {
        fn extract(
            &self,
            _: &dyn ClientHost,
            _: &mod_api::ItemStack,
            _: &Tag,
            _: &SpecialModelContext,
        ) -> Result<Vec<SpecialModelPart>, String> {
            unreachable!("the tests answer through `guarded`'s closure")
        }
    }

    fn renderer() -> Resolved<dyn SpecialModelRenderer> {
        Resolved::new(Arc::new(Unused))
    }

    /// A renderer's parts reach the resolver posed by their row-major transform and tinted.
    #[test]
    fn parts_reach_the_resolver_posed_and_tinted() {
        let parts = guarded("t:pan", &renderer(), || {
            Ok(vec![
                SpecialModelPart::new("t:item/a")
                    .translated([1.0, 2.0, 3.0])
                    .with_tints(vec![0xFF11_2233]),
            ])
        })
        .unwrap();
        assert_eq!(parts[0].content, SpecialContent::Model("t:item/a".into()));
        assert_eq!(parts[0].pose.0, Mat4::translation([1.0, 2.0, 3.0]).0);
        assert_eq!(parts[0].tints, [0xFF11_2233_u32 as i32]);
    }

    #[test]
    fn unreadable_answers_are_retained_only_for_the_frame() {
        let answers = Mutex::<Answers>::default();
        let stack = StackView::new("t:pan", 1);
        let asked = std::cell::Cell::new(0);
        let draw = || {
            answer(&answers, 1, &stack, None, || {
                asked.set(asked.get() + 1);
                None
            })
        };
        assert!(draw().is_none());
        assert!(draw().is_none());
        assert_eq!(asked.get(), 1);
        answers.lock().unwrap().begin_frame(1);
        assert!(draw().is_none());
        assert_eq!(asked.get(), 2);
    }

    /// An unchanged stack is asked once for as long as every frame draws it, however many stacks
    /// a model node draws; another stack, or the same stack in another node, is asked anew. A
    /// stack a frame didn't draw, or any after a reload, is asked again.
    #[test]
    fn an_unchanged_stack_is_extracted_once_across_frames() {
        const VIEW: usize = 1;
        let answers = Mutex::<Answers>::default();
        let asked = std::cell::Cell::new(0);
        let draw = |node: usize, stack: &StackView| {
            answer(&answers, node, stack, None, || {
                asked.set(asked.get() + 1);
                Some(PreparedParts(Arc::new(stack.count)))
            })
            .unwrap()
        };
        let count = |prepared: PreparedParts| *prepared.0.downcast_ref::<i32>().unwrap();
        // Nodes far from any real `config` address; 100 stacks of the pan in a stable order,
        // more than any fixed count would keep.
        let (pan, pot) = (1, 2);
        let stacks: Vec<StackView> = (1..=100).map(|n| StackView::new("t:pan", n)).collect();
        let first: Vec<PreparedParts> = stacks.iter().map(|s| draw(pan, s)).collect();
        for _frame in 0..10 {
            answers.lock().unwrap().begin_frame(VIEW);
            for (stack, first) in stacks.iter().zip(&first) {
                assert_eq!(&draw(pan, stack), first);
            }
        }
        assert_eq!(asked.get(), 100);
        // An equal stack read anew (the game builds its view every draw) is the same stack.
        let mut named = StackView::new("t:pan", 1);
        named.patch.insert(
            "minecraft:custom_name".into(),
            Some(serde_json::json!({"text": "Pan", "bold": true})),
        );
        let mut renamed = StackView::new("t:pan", 1);
        renamed.patch.insert(
            "minecraft:custom_name".into(),
            Some(serde_json::json!({"bold": true, "text": "Pan"})),
        );
        assert_eq!(draw(pan, &named), draw(pan, &renamed));
        assert_eq!(asked.get(), 101);
        assert_eq!(count(draw(pot, &stacks[0])), 1);
        assert_eq!(asked.get(), 102);
        // The next frame draws only the first stack: the others are asked again after it.
        answers.lock().unwrap().begin_frame(VIEW);
        assert_eq!(draw(pan, &stacks[0]), first[0]);
        answers.lock().unwrap().begin_frame(VIEW);
        assert_eq!(draw(pan, &stacks[0]), first[0]);
        assert_eq!(asked.get(), 102);
        assert_eq!(count(draw(pan, &stacks[1])), 2);
        assert_eq!(asked.get(), 103);
        assert_eq!(answers.lock().unwrap().kept.len(), 2);
        // When two views draw in turn, neither view's stack is
        // dropped by the other's frame.
        for _frame in 0..10 {
            answers.lock().unwrap().begin_frame(VIEW);
            assert_eq!(draw(pan, &stacks[0]), first[0]);
            answers.lock().unwrap().begin_frame(VIEW + 1);
            assert_eq!(count(draw(pan, &stacks[2])), 3);
        }
        assert_eq!(asked.get(), 104);
        answers.lock().unwrap().kept.clear();
        draw(pan, &stacks[0]);
        assert_eq!(asked.get(), 105);
    }

    /// A `Frame` answer is asked again for another frame, display context or time, and shared
    /// only by an equal stack drawn the same way in the same frame, a refusal included.
    #[test]
    fn a_frame_answer_is_kept_for_its_frame_display_and_time_only() {
        let answers = Mutex::<Answers>::default();
        answers.lock().unwrap().begin_frame(1);
        let stack = StackView::new("t:pan", 1);
        let asked = std::cell::Cell::new(0);
        let timing = |display, game_time, partial: f32| FrameTiming {
            display,
            game_time: Some(game_time),
            partial_tick: partial.to_bits(),
        };
        let draw = |timing: FrameTiming, stack: &StackView| {
            answer(&answers, 1, stack, Some(timing), || {
                asked.set(asked.get() + 1);
                (stack.count != 9).then(|| PreparedParts(Arc::new(asked.get())))
            })
        };
        let hand = timing(ItemDisplayContext::FirstPersonRightHand, 100, 0.25);
        let first = draw(hand, &stack);
        assert_eq!(draw(hand, &stack), first);
        assert_eq!(asked.get(), 1);
        assert_ne!(
            draw(timing(ItemDisplayContext::Gui, 100, 0.25), &stack),
            first
        );
        assert_ne!(
            draw(
                timing(ItemDisplayContext::FirstPersonRightHand, 100, 0.5),
                &stack
            ),
            first
        );
        assert_ne!(
            draw(
                timing(ItemDisplayContext::FirstPersonRightHand, 101, 0.25),
                &stack
            ),
            first
        );
        assert_ne!(draw(hand, &StackView::new("t:pan", 2)), first);
        assert_eq!(asked.get(), 5);
        // A refusal is kept for the frame too.
        let refused = StackView::new("t:pan", 9);
        assert_eq!(draw(hand, &refused), None);
        assert_eq!(draw(hand, &refused), None);
        assert_eq!(asked.get(), 6);
        // The next frame asks again, though nothing about the stack changed.
        answers.lock().unwrap().begin_frame(1);
        assert_ne!(draw(hand, &stack), first);
        assert_eq!(asked.get(), 7);
        assert!(answers.lock().unwrap().kept.is_empty());
    }

    /// An `Err`, a panic, 257 parts and a transform that isn't finite each refuse the answer
    /// whole, and the renderer isn't asked again until resources reload.
    #[test]
    fn a_failing_renderer_is_refused_whole_and_asked_no_more_until_reload() {
        let nan = SpecialModelPart::new("t:item/a").translated([f32::NAN, 0.0, 0.0]);
        let failures: [Box<dyn Fn() -> Result<Vec<SpecialModelPart>, String>>; 4] = [
            Box::new(|| Err("no pan".into())),
            Box::new(|| panic!("the pan fell")),
            Box::new(|| {
                Ok(vec![
                    SpecialModelPart::new("t:item/a");
                    MAX_SPECIAL_MODEL_PARTS + 1
                ])
            }),
            Box::new(move || Ok(vec![SpecialModelPart::new("t:item/b"), nan.clone()])),
        ];
        for fail in failures {
            let resolved = renderer();
            assert_eq!(guarded("t:pan", &resolved, fail), None);
            let mut asked = false;
            assert_eq!(
                guarded("t:pan", &resolved, || {
                    asked = true;
                    Ok(Vec::new())
                }),
                None
            );
            assert!(!asked, "a failed renderer is asked again before reload");
            resolved.failed.store(false, Ordering::Relaxed);
            assert_eq!(
                guarded("t:pan", &resolved, || Ok(Vec::new())),
                Some(Vec::new())
            );
        }
        let full = vec![SpecialModelPart::new("t:item/a"); MAX_SPECIAL_MODEL_PARTS];
        assert_eq!(
            guarded("t:pan", &renderer(), || Ok(full)).map(|parts| parts.len()),
            Some(MAX_SPECIAL_MODEL_PARTS)
        );
    }
}
