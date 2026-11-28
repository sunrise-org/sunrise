//! The server's resident display data and its `SynchedEntityData` projection.
#[cfg(test)]
mod tests;

use crate::{
    items::ItemStack,
    scoreboard::{component_tag, tag_to_json},
    text::Text,
};
use mod_api::{Billboard, ItemDisplayContext, TextAlignment, Transformation};
use std::sync::Arc;
use sunrise_core::nbt::{Tag, clone_tag};
use sunrise_protocol::{
    entity_data::{Value, serializer},
    item::Stack,
};
use sunrise_world::registry::BlockStateId;

/// The three display subclasses have different entity-data accessors after id 22.
#[derive(Clone, Debug, PartialEq)]
pub enum DisplayContents {
    Block(BlockStateId),
    Item {
        stack: Option<Arc<ItemStack>>,
        context: ItemDisplayContext,
    },
    Text(DisplayText),
}

/// A decoded component tree in its NBT data model. Unlike the JSON view, it
/// retains the tag types inside payloads such as `HoverEvent.ShowItem` patches.
#[derive(Debug, PartialEq)]
pub struct DisplayComponent(Tag);

impl DisplayComponent {
    pub fn from_nbt(tag: &Tag) -> Option<Self> {
        match tag {
            Tag::String(_) => {}
            Tag::Compound(fields) if !fields.is_empty() => {}
            Tag::List(items) if !items.is_empty() => {}
            _ => return None,
        }
        Some(Self(clone_tag(tag)))
    }

    pub fn from_text(text: &Text) -> Self {
        Self(component_tag(&text.component_view()))
    }

    pub fn to_nbt(&self) -> Tag {
        clone_tag(&self.0)
    }

    pub fn project_text(&self) -> Text {
        Text::from(tag_to_json(&self.0))
    }

    fn is_empty(&self) -> bool {
        matches!(&self.0, Tag::String(text) if text.0.is_empty())
    }
}

impl Default for DisplayComponent {
    fn default() -> Self {
        Self(Tag::String("".into()))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DisplayText {
    pub component: Arc<DisplayComponent>,
    pub line_width: i32,
    pub background: i32,
    pub opacity: i8,
    pub shadow: bool,
    pub see_through: bool,
    pub default_background: bool,
    pub alignment: TextAlignment,
}

impl Default for DisplayText {
    fn default() -> Self {
        Self {
            component: Arc::default(),
            line_width: 200,
            background: 0x4000_0000,
            opacity: -1,
            shadow: false,
            see_through: false,
            default_background: false,
            alignment: TextAlignment::Center,
        }
    }
}

impl DisplayText {
    fn flags(&self) -> i8 {
        i8::from(self.shadow)
            | (i8::from(self.see_through) << 1)
            | (i8::from(self.default_background) << 2)
            | if self.alignment == TextAlignment::Left {
                8
            } else if self.alignment == TextAlignment::Right {
                16
            } else {
                0
            }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DisplayState {
    pub transformation: Transformation,
    pub billboard: Billboard,
    pub brightness: Option<(u8, u8)>,
    pub view_range: f32,
    pub shadow_radius: f32,
    pub shadow_strength: f32,
    pub width: f32,
    pub height: f32,
    pub glow_color_override: i32,
    pub interpolation_duration: i32,
    pub start_interpolation: i32,
    pub teleport_duration: i32,
    pub contents: DisplayContents,
}

impl DisplayState {
    /// Vanilla `defineSynchedData`, with the subclass's initial contents.
    pub fn new(contents: DisplayContents) -> Self {
        Self {
            transformation: Transformation::IDENTITY,
            billboard: Billboard::Fixed,
            brightness: None,
            view_range: 1.0,
            shadow_radius: 0.0,
            shadow_strength: 1.0,
            width: 0.0,
            height: 0.0,
            glow_color_override: -1,
            interpolation_duration: 0,
            start_interpolation: 0,
            teleport_duration: 0,
            contents,
        }
    }

    /// `Display.addAdditionalSaveData`: save the settings as the codecs read
    /// them, including defaults. The raw start delay is only used by tracking.
    pub fn write_nbt(
        &self,
        tag: &mut Tag,
        registry: &sunrise_world::registry::BlockRegistry,
        items: &crate::items::Items,
    ) {
        let Tag::Compound(fields) = tag else { return };
        let real = |v: f32| Tag::Float(if v == 0.0 { 0 } else { v.to_bits() });
        let floats = |parts: &[f32]| Tag::List(parts.iter().copied().map(real).collect());
        let t = self.transformation;
        let transform = Tag::compound([
            ("translation", floats(&t.translation)),
            ("left_rotation", floats(&t.left_rotation)),
            ("scale", floats(&t.scale)),
            ("right_rotation", floats(&t.right_rotation)),
        ]);
        for (key, value) in [
            ("transformation", transform),
            ("billboard", Tag::String(self.billboard.name().into())),
            (
                "interpolation_duration",
                Tag::Int(self.interpolation_duration),
            ),
            ("teleport_duration", Tag::Int(self.teleport_duration)),
            ("view_range", real(self.view_range)),
            ("shadow_radius", real(self.shadow_radius)),
            ("shadow_strength", real(self.shadow_strength)),
            ("width", real(self.width)),
            ("height", real(self.height)),
            ("glow_color_override", Tag::Int(self.glow_color_override)),
        ] {
            fields.insert(key.into(), value);
        }
        fields.remove(&"start_interpolation".into());
        if let Some((block, sky)) = self.brightness {
            fields.insert(
                "brightness".into(),
                Tag::compound([
                    ("block", Tag::Int(i32::from(block))),
                    ("sky", Tag::Int(i32::from(sky))),
                ]),
            );
        } else {
            fields.remove(&"brightness".into());
        }
        match &self.contents {
            DisplayContents::Block(state) => {
                fields.insert(
                    "block_state".into(),
                    crate::entity_nbt::state_to_nbt(registry, *state),
                );
            }
            DisplayContents::Item { stack, context } => {
                fields.insert("item_display".into(), Tag::String(context.name().into()));
                if let Some(stack) = stack {
                    fields.insert(
                        "item".into(),
                        sunrise_items::item_nbt::item_to_nbt(stack, items),
                    );
                } else {
                    fields.remove(&"item".into());
                }
            }
            DisplayContents::Text(text) => {
                for (key, value) in [
                    ("text", text.component.to_nbt()),
                    ("line_width", Tag::Int(text.line_width)),
                    ("background", Tag::Int(text.background)),
                    ("text_opacity", Tag::Byte(text.opacity)),
                    ("shadow", Tag::Byte(i8::from(text.shadow))),
                    ("see_through", Tag::Byte(i8::from(text.see_through))),
                    (
                        "default_background",
                        Tag::Byte(i8::from(text.default_background)),
                    ),
                    ("alignment", Tag::String(text.alignment.name().into())),
                ] {
                    fields.insert(key.into(), value);
                }
            }
        }
    }

    /// `SynchedEntityData.getNonDefaultValues` on pairing, or changed fields
    /// (including a return to the default) on an update. Item components are
    /// encoded with the connection's registries only if the stack is sent.
    pub fn append_metadata(
        &self,
        previous: Option<&Self>,
        force_start: bool,
        encode_stack: impl FnOnce(&ItemStack) -> Option<Stack>,
        out: &mut Vec<(u8, i32, Value)>,
    ) {
        let mut put = |id, kind, value| out.push((id, kind, value));
        let defaults = Self::new(DisplayContents::Block(BlockStateId(0)));
        let before = previous.unwrap_or(&defaults);
        // Java Float.equals and JOML's component equality use floatToIntBits.
        let same = |a: f32, b: f32| a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan());
        let t = self.transformation;
        let from = before.transformation;
        for (id, now, old) in [
            (8, self.start_interpolation, before.start_interpolation),
            (
                9,
                self.interpolation_duration,
                before.interpolation_duration,
            ),
            (10, self.teleport_duration, before.teleport_duration),
        ] {
            if now != old || (id == 8 && force_start) {
                put(id, serializer::INT, Value::Int(now));
            }
        }
        for (id, kind, now, old) in [
            (
                11,
                serializer::VECTOR3,
                t.translation.as_slice(),
                from.translation.as_slice(),
            ),
            (
                12,
                serializer::VECTOR3,
                t.scale.as_slice(),
                from.scale.as_slice(),
            ),
            (
                13,
                serializer::QUATERNION,
                t.left_rotation.as_slice(),
                from.left_rotation.as_slice(),
            ),
            (
                14,
                serializer::QUATERNION,
                t.right_rotation.as_slice(),
                from.right_rotation.as_slice(),
            ),
        ] {
            if !now.iter().zip(old).all(|(&a, &b)| same(a, b)) {
                put(id, kind, Value::Floats(now.to_vec()));
            }
        }
        if self.billboard != before.billboard {
            put(15, serializer::BYTE, Value::Byte(self.billboard as i8));
        }
        if self.brightness != before.brightness {
            let light = self.brightness.map_or(-1, |(block, sky)| {
                (i32::from(block) << 4) | (i32::from(sky) << 20)
            });
            put(16, serializer::INT, Value::Int(light));
        }
        for (id, now, old) in [
            (17, self.view_range, before.view_range),
            (18, self.shadow_radius, before.shadow_radius),
            (19, self.shadow_strength, before.shadow_strength),
            (20, self.width, before.width),
            (21, self.height, before.height),
        ] {
            if !same(now, old) {
                put(id, serializer::FLOAT, Value::Float(now));
            }
        }
        if self.glow_color_override != before.glow_color_override {
            put(22, serializer::INT, Value::Int(self.glow_color_override));
        }
        match &self.contents {
            DisplayContents::Block(state) => {
                let before = match before.contents {
                    DisplayContents::Block(state) => state,
                    _ => BlockStateId(0),
                };
                if *state != before {
                    put(23, serializer::BLOCK_STATE, Value::Id(state.0 as i32));
                }
            }
            DisplayContents::Item { stack, context } => {
                let (old_stack, old_context) = match &before.contents {
                    DisplayContents::Item { stack, context } => (stack.as_ref(), *context),
                    _ => (None, ItemDisplayContext::None),
                };
                let same_stack = match (stack.as_ref(), old_stack) {
                    (Some(new), Some(old)) => Arc::ptr_eq(new, old) || **new == **old,
                    (None, None) => true,
                    _ => false,
                };
                if !same_stack {
                    match stack.as_deref() {
                        Some(stack) => {
                            if let Some(encoded) = encode_stack(stack) {
                                put(23, serializer::ITEM_STACK, Value::Item(Some(encoded)));
                            }
                        }
                        None => put(23, serializer::ITEM_STACK, Value::Item(None)),
                    }
                }
                if *context != old_context {
                    put(24, serializer::BYTE, Value::Byte(*context as i8));
                }
            }
            DisplayContents::Text(text) => {
                let before = match &before.contents {
                    DisplayContents::Text(text) => Some(text),
                    _ => None,
                };
                if before.map_or_else(
                    || !text.component.is_empty(),
                    |old| {
                        !Arc::ptr_eq(&old.component, &text.component)
                            && old.component != text.component
                    },
                ) {
                    put(
                        23,
                        serializer::COMPONENT,
                        Value::Text(text.component.to_nbt()),
                    );
                }
                if text.line_width != before.map_or(200, |text| text.line_width) {
                    put(24, serializer::INT, Value::Int(text.line_width));
                }
                if text.background != before.map_or(0x4000_0000, |text| text.background) {
                    put(25, serializer::INT, Value::Int(text.background));
                }
                if text.opacity != before.map_or(-1, |text| text.opacity) {
                    put(26, serializer::BYTE, Value::Byte(text.opacity));
                }
                let flags = text.flags();
                if flags != before.map_or(0, DisplayText::flags) {
                    put(27, serializer::BYTE, Value::Byte(flags));
                }
            }
        }
    }
}
