# Measuring styled text outside drawing

Measure styled text without a draw context. Use `ClientHost::text_width(&Text)` and `text_split(&Text, width)`:
these are reads of the client's active font in unscaled GUI pixels, also usable
in client setup.

| Method on `ClientHost` | Result |
| --- | --- |
| `text_width(&Text)` | `Result<i32, TextFormError>`: styled width in GUI pixels. |
| `text_split(&Text, i32)` | `Result<Vec<Text>, TextFormError>`: wrapped, styled lines. |
| `text_visual_order(&Text)` | `Result<Vec<VisualTextCharacter>, TextFormError>`: styled characters in display order. |

Inside a drawing hook, `DrawContext::text_lines(&Text, max_width, TextAlign, scale)` wraps and
places the same way and adds what a label needs: each line's `Text`, its `at` position inside the
width and its width, plus the block's `width` and `height` — all answered in scaled GUI pixels, the
text wrapping at `max_width / scale`. It refuses invalid widths and scales and oversized layouts
with a `TextLinesRefusal`. See [Screens](screens.md#labels-wrapped-text-without-a-widget).

Construct an owned `Text` with its content, style and children. A bridge with a
flat component representation can build a `TextForm` and call
`Text::try_from(form)` once at its boundary; the host reads accept that validated
`Text`, not JSON or engine text types. Both methods return a `TextFormError` if
the supplied component cannot be represented at the engine boundary.

Width is the same styled and translated width used by the engine's tooltip
layout. Wrapping follows `Font.split(FormattedText, width)`: it keeps styles,
breaks at newlines and fitting spaces, and uses the vanilla one-pixel minimum
for widths below one. Returned lines are API-owned `Text` values. Translation
uses the current client language; a later language or resource-pack font change
is reflected by the next call, so do not retain dimensions across reloads.

NeoForge's `ClientTooltipComponent` receives a `Font` in its width and drawing
methods. These reads use that same role—the active client font—without passing
an engine font object or requiring a `DrawContext`. They do not draw anything
and do not expose a full rendering context.

Chat and signs can [substitute inline images](dynamic-textures.md#inline-images-in-chat-and-signs) before their own
layout. The reads above measure the component you pass, without running that presentation hook.

## Resolved strings

For vanilla `Component.getString()` behavior, use
`host.text_string(&Text) -> Result<String, TextFormError>` on either side,
including setup. It uses the same component conversion as
measurement: translation arguments, keybinds and siblings resolve on each call.
Styles do not add characters; literal legacy `§` formatting codes remain in the
string. Invalid components return the same boundary errors as the other text reads.

- **Client and integrated server:** both use the local player's selected language,
  active resource packs (including client mod translations) and key bindings.
  An integrated server does not resolve against a LAN guest's language.
- **Dedicated server:** starts with built-in vanilla and Sunrise English; world
  and data-load listeners still see those built-ins. Once the world has loaded,
  just before `server_starting`, it loads `assets/<namespace>/lang/en_us.json` from
  enabled mods and the data packs selected for this world. Sunrise visits
  namespaces in lexical order and, within each namespace, reads the original
  files in pack priority order (mods first). Later values win. An invalid
  entry leaves the earlier values intact and stops the remaining files in
  that namespace. Vanilla's deprecated keys are applied at the end. Neither
  client resource packs nor later data-pack reloads affect the snapshot.
  Keybind components give their literal IDs.

The pack-discovery host has no game text backend. `text_string` there returns
`TextFormError::Unavailable`.

A missing key uses its component fallback, or the key itself without a
fallback. On a dedicated server, a pack language file may also supply an
object or array representing a component: it replaces the whole translation,
including its arguments and fallback. A recursive reference to the same
language entry resolves to no text.

Send an unresolved component to a player when the result should use that player's
language. Do not cache resolved strings across language changes.

## Visual order

For vanilla `Component.getVisualOrderText()` behavior, use
`ClientHost::text_visual_order(&Text) -> Result<Vec<VisualTextCharacter>, TextFormError>`.
Each API-owned Wire record contains a Unicode scalar (`scalar`), its full
resolved `TextStyle`, and the sink's `source_index`. Characters are already in
visual order, including Arabic shaping and mirrored punctuation; do not apply
bidi a second time. This read uses the engine's existing language and bidi
pipeline, is available outside drawing, including client setup.

Sink indices are UTF-16 offsets local to a constant-style substring within a
directional run, not global component positions. For example, `A😀B` yields
indices `0, 1, 3`; Hebrew `שלום` yields `3, 2, 1, 0`. A style or bidi run boundary
restarts the index. Shaping can shorten the string, so these are not guaranteed
to identify original input scalars. Use the records to re-layout existing tooltip
lines without losing click, hover, font or other style information. Like the
other reads, invalid component input returns its boundary error.

Action keybind IDs such as `key.jump` resolve through the client's current key
binding to their assigned physical key display name, matching vanilla's `KeybindContents`.
Physical key names resolve normally.
