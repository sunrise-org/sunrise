# Wire types

`mod_api::wire::{Wire, Reader, Writer, WireError}` defines the codec used by
[typed payload tokens](../how-to/client/networking.md#typed-payloads) and
[recipe placement and recipe-book sync](../crafting-recipe-kinds.md). Typed payload bodies
use `PayloadCodec`, not `Wire`. Derive `mod_api::Wire` explicitly on a type that needs
this encoding. The build never adds the derive. Every field of a derived type must
also implement `Wire`.

```rust,ignore
#[derive(mod_api::Wire)]
pub struct ChemicalAmount {
    pub chemical: String,
    pub amount: u64,
}
```

Records encode fields in declaration order. Enums encode a case index followed by
its fields. Lengths and numbers are little-endian; collections are length-prefixed.
Decoding returns `WireError` for invalid data rather than accepting an unknown tag,
invalid UTF-8 or an unsupported length.

## Nesting and stacks

Structural decoding and encoding allow **32** nested records, payload-carrying
variants and `Box`/`Arc` values (`MAX_DEPTH`). Collection iteration, `Option`
and tuples do not consume a level. The reader returns `WireError::Depth` before
entering level 33; the writer records the same error. Always check `Writer::error`
before sending its bytes. This is a transport budget for unbounded mod-defined
trees, including ingredients, item places, loot vehicles and recipe slot displays.

Vanilla trees are not restricted by that structural budget: NBT `Tag` uses an
explicit work stack with **512** nested containers; `Text` crosses as the existing
validated flat `TextForm` table, preserving its **512** component-depth contract.
Text's layout identity is the flat table's identity.

JSON-backed `serde_json::Value`, JSON maps and `ModelPart` also refuse beyond
32 JSON containers. Their encoders preflight the in-memory tree iteratively
before serde recursion; their decoders scan the JSON's nesting outside quoted
strings before parsing. A model part's JSON includes object, children-list and
named-child-tuple containers, so one part can consume several JSON levels.

Hand-written recursive `Wire` implementations must enforce their own stack contract; the
reader cannot police code outside its codec.

Encoding borrows the input (`writer.put(&value)`); the caller still owns it.
Constructing an arbitrarily deep Rust tree and then dropping it can overflow a
thread independently of Wire. To dispose of deeply nested NBT, pop tags from
an explicit work stack. Push the children of each `Tag::List` and the values
from `Compound::into_values()` for each `Tag::Compound`; drop leaf tags as you go.

## Checked representations

Use `#[wire(via = String)]` (or another `Wire` type) when a checked constructor owns
validation. Implement `From<&YourType> for String` and `TryFrom<String> for YourType`.
The derive writes the representation and maps a failed conversion to
`WireError::Invalid`. Its layout is the representation's layout.

## Layout identity

A type's schema digest describes the layout that `Wire` reads and writes: fields, enum cases and
child types. The derive declares the shape from the Rust fields. A
hand-written implementation declares it with `layout_fields!` (its fields) or `layout_via!(T)`
(the representation it encodes as). Container shapes follow the encoding: `Vec<T>` and
`VecDeque<T>` agree, as do `String` and `Box<str>`; `Box<T>` is transparent. The digest does
not include the Rust type's path. The wire format has no compatibility versions or default fields.

## Hand-written implementations

The trait is unsealed. A hand-written implementation must supply `put`, `take` and `MIN`,
and declare its shape beside the impl with `layout_fields!` or `layout_via!`; the derive
handles this automatically and is preferred.

A hand decoder owns tag validation and must use `Reader::nested` for recursive
values. `Reader` checks byte bounds and UTF-8, but cannot enforce invariants that
an implementation neglects. Keep validation at the checked constructor boundary.
