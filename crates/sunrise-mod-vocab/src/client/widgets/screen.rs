//! Listeners of a stock screen's current build.

use super::{Widget, char_len, is_allowed_chat_character, utf16_len};
use crate::{ClientHost, Text};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{any::Any, sync::Arc};

/// An opaque reference to a widget in one stock screen build. Removing the
/// listener, closing the screen or rebuilding it invalidates this reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScreenWidgetRef(u64);

/// The kind of control in a stock screen's listener list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScreenWidgetKind {
    /// A pressable button, including an icon or text button.
    Button,
    /// A single-line edit box.
    TextField,
}

/// Read-only description of one listener. Bounds are absolute GUI pixels.
#[derive(Clone, Copy, Debug)]
pub struct ScreenWidgetView<'a> {
    /// Reference to pass to [`ScreenWidgets::remove_listener`].
    pub listener: ScreenWidgetRef,
    /// The type of control.
    pub kind: ScreenWidgetKind,
    /// `[x, y, width, height]`, in absolute GUI pixels.
    pub bounds: [f32; 4],
    /// Its label, where the control has one.
    pub label: Option<&'a Text>,
}

/// Why an edit to a screen's listener list was refused. A refusal does not
/// change the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScreenWidgetError {
    /// This screen does not expose its listeners yet.
    UnsupportedScreen,
    /// Bounds must intersect the build viewport, with positive extents no larger than it.
    InvalidBounds,
    /// A label or initial text edit is malformed.
    InvalidText,
    /// A button requires a nonempty action ID and callback; a text field takes neither.
    InvalidAction,
    /// This mod already registered the action ID in this build.
    DuplicateAction,
    /// The reference is not in the current staged list.
    UnknownListener,
}

/// A native callback attached to one stock-screen button. The engine passes the
/// registering mod's action ID and client host on each enabled activation.
pub type ScreenWidgetPress = fn(&mut dyn ClientHost, &str);

#[doc(hidden)]
#[derive(Clone)]
pub struct ScreenWidgetAction {
    pub owner: &'static str,
    pub id: Arc<str>,
    pub press: ScreenWidgetPress,
}

#[doc(hidden)]
#[derive(Clone)]
pub struct ScreenWidgetEntry {
    reference: ScreenWidgetRef,
    kind: ScreenWidgetKind,
    bounds: [f32; 4],
    label: Option<Text>,
    pub stock: Option<u8>,
    pub widget: Option<Widget>,
    pub action: Option<ScreenWidgetAction>,
    pub render_label: Option<Arc<dyn Any + Send + Sync>>,
}

type PrepareLabel = fn(&Text) -> Result<Arc<dyn Any + Send + Sync>, crate::text::TextFormError>;

/// Engine-owned staging. Only dispatch can lend an editing view of it.
#[doc(hidden)]
pub struct ScreenWidgetBuild {
    entries: Vec<ScreenWidgetEntry>,
    actions: Vec<(&'static str, Arc<str>)>,
    viewport: Option<[f32; 2]>,
    next_listener: &'static AtomicU64,
    prepare_label: PrepareLabel,
}

/// The staged widget listeners of an initialized stock screen, available only
/// during `screen_init_post`. Stock listeners come first and edits from earlier
/// successful callbacks are visible. Only `title` and `paused` expose listeners.
pub struct ScreenWidgets<'a> {
    build: &'a mut ScreenWidgetBuild,
    owner: &'static str,
}

impl ScreenWidgets<'_> {
    /// Lists the current staged listeners in screen order.
    pub fn listeners(&self) -> impl ExactSizeIterator<Item = ScreenWidgetView<'_>> {
        self.build.entries.iter().map(|entry| ScreenWidgetView {
            listener: entry.reference,
            kind: entry.kind,
            bounds: entry.bounds,
            label: entry.label.as_ref(),
        })
    }

    /// Appends a widget. Pass `Some((id, callback))` for a button, or `None`
    /// for a text field. Action IDs are private to the registering mod and
    /// cannot be reused in the same build, even after a removal.
    pub fn add_listener(
        &mut self,
        widget: Widget,
        action: Option<(&str, ScreenWidgetPress)>,
    ) -> Result<ScreenWidgetRef, ScreenWidgetError> {
        let [width, height] = self
            .build
            .viewport
            .ok_or(ScreenWidgetError::UnsupportedScreen)?;
        let bounds @ [x, y, w, h] = widget.rect();
        if !bounds.into_iter().all(f32::is_finite)
            || !(0.0 < w && w <= width && 0.0 < h && h <= height)
            || x + w <= 0.0
            || y + h <= 0.0
            || x >= width
            || y >= height
        {
            return Err(ScreenWidgetError::InvalidBounds);
        }
        let owner = self.owner;
        let (kind, label, action, render_label) = match (&widget, action) {
            (Widget::Button(button), Some((id, press))) if !id.trim().is_empty() => {
                let render_label = (self.build.prepare_label)(&button.label)
                    .map_err(|_| ScreenWidgetError::InvalidText)?;
                if self
                    .build
                    .actions
                    .iter()
                    .any(|(mod_id, action)| *mod_id == owner && action.as_ref() == id)
                {
                    return Err(ScreenWidgetError::DuplicateAction);
                }
                let id: Arc<str> = id.into();
                self.build.actions.push((owner, id.clone()));
                (
                    ScreenWidgetKind::Button,
                    Some(button.label.clone()),
                    Some(ScreenWidgetAction { owner, id, press }),
                    Some(render_label),
                )
            }
            (Widget::TextField(field), None) => {
                let len = char_len(field.value());
                if field.edit.multiline
                    || field.edit.cursor > len
                    || field.edit.highlight > len
                    || utf16_len(field.value()) > field.edit.max_length
                    || !field.value().chars().all(is_allowed_chat_character)
                {
                    return Err(ScreenWidgetError::InvalidText);
                }
                (
                    ScreenWidgetKind::TextField,
                    Some(Text::literal(field.message.clone())),
                    None,
                    None,
                )
            }
            _ => return Err(ScreenWidgetError::InvalidAction),
        };
        let reference = ScreenWidgetRef(self.build.next_listener.fetch_add(1, Ordering::Relaxed));
        self.build.entries.push(ScreenWidgetEntry {
            reference,
            kind,
            bounds,
            label,
            stock: None,
            widget: Some(widget),
            action,
            render_label,
        });
        Ok(reference)
    }

    /// Removes a listener, including a stock control, from drawing, input,
    /// focus and narration for this build. References from other builds refuse.
    pub fn remove_listener(&mut self, listener: ScreenWidgetRef) -> Result<(), ScreenWidgetError> {
        if self.build.viewport.is_none() {
            return Err(ScreenWidgetError::UnsupportedScreen);
        }
        let index = self
            .build
            .entries
            .iter()
            .position(|entry| entry.reference == listener)
            .ok_or(ScreenWidgetError::UnknownListener)?;
        self.build.entries.remove(index);
        Ok(())
    }
}

impl ScreenWidgetBuild {
    pub fn __new(
        viewport: Option<[f32; 2]>,
        next_listener: &'static AtomicU64,
        prepare_label: PrepareLabel,
    ) -> Self {
        Self {
            entries: Vec::new(),
            actions: Vec::new(),
            viewport,
            next_listener,
            prepare_label,
        }
    }

    #[doc(hidden)]
    pub fn __stock(&mut self, id: u8, bounds: [f32; 4], label: Option<Text>) {
        self.entries.push(ScreenWidgetEntry {
            reference: ScreenWidgetRef(self.next_listener.fetch_add(1, Ordering::Relaxed)),
            kind: ScreenWidgetKind::Button,
            bounds,
            label,
            stock: Some(id),
            widget: None,
            action: None,
            render_label: None,
        });
    }

    #[doc(hidden)]
    pub(crate) fn __checkpoint(&self) -> (Vec<ScreenWidgetEntry>, usize) {
        (self.entries.clone(), self.actions.len())
    }

    #[doc(hidden)]
    pub(crate) fn __rollback(&mut self, (entries, actions): (Vec<ScreenWidgetEntry>, usize)) {
        self.entries = entries;
        self.actions.truncate(actions);
    }

    pub(crate) fn __edit(&mut self, owner: &'static str) -> ScreenWidgets<'_> {
        ScreenWidgets { build: self, owner }
    }

    #[doc(hidden)]
    pub fn __finish(self) -> Vec<ScreenWidgetEntry> {
        self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::widgets::{Button, TextField};

    fn press(_: &mut dyn ClientHost, _: &str) {}

    #[test]
    fn malformed_edits_leave_the_staged_list_unchanged() {
        static NEXT_LISTENER: AtomicU64 = AtomicU64::new(0);
        let mut staged =
            ScreenWidgetBuild::__new(Some([320.0, 240.0]), &NEXT_LISTENER, |_| Ok(Arc::new(())));
        let mut build = staged.__edit("first");
        let button = |rect| Widget::Button(Button::new(rect, Text::literal("Example")));
        let binding = Some(("settings", press as ScreenWidgetPress));
        assert_eq!(
            build.add_listener(button([1.0, 4.0, f32::MAX, 20.0]), binding),
            Err(ScreenWidgetError::InvalidBounds)
        );
        assert_eq!(
            build.add_listener(button([1.0, 4.0, 80.0, 20.0]), None),
            Err(ScreenWidgetError::InvalidAction)
        );
        let mut field = TextField::new([10.0, 10.0, 30.0, 20.0], 1);
        field.edit.value = "😀".into();
        assert_eq!(
            build.add_listener(Widget::TextField(field), None),
            Err(ScreenWidgetError::InvalidText)
        );
        assert_eq!(build.listeners().len(), 0);
        let listener = build
            .add_listener(button([1.0, 4.0, 80.0, 20.0]), binding)
            .unwrap();
        assert_eq!(
            build.add_listener(button([2.0, 4.0, 80.0, 20.0]), binding),
            Err(ScreenWidgetError::DuplicateAction)
        );
        assert_eq!(build.listeners().next().unwrap().listener, listener);
        build.remove_listener(listener).unwrap();
        assert_eq!(
            build.remove_listener(listener),
            Err(ScreenWidgetError::UnknownListener)
        );
    }
}
