# Pressing gameplay key mappings

A client mod can press the player's vanilla gameplay mappings as their keys do: hold one down and let
it go, or click it. The game then acts through its own key handling, with every check it makes for the
player's keys, so the server sees only the packets of a normal client, at the vanilla moments.

```rust
// Hold sprint, and attack once on the next tick.
host.set_mapping_down("key.sprint", true)?;
host.click_mapping("key.attack")?;
```

`ClientHost::set_mapping_down(key, down)` sets the held state (`KeyMapping.setDown`), without the click a
press adds. For a hold-style mapping, `false` releases the mapping even if the player is holding
its key or controller button; `true` holds the mapping without creating a second device hold.
Neither changes the physical input, and the next input for that mapping takes over again.
`ClientHost::click_mapping(key)` adds one click (`KeyMapping.click`), without holding the mapping.
A press of the key itself does both; a mod that wants a held attack that also starts at once calls both.

Queued actions are taken oldest first, at most **64 entries per queue per drain**, with at most
**1024 pending entries**. A burst spills into later drains, normally later client ticks, rather than
all acting on the next tick. The hotbar differs: each number key keeps its own counter of up to
**1024 pending clicks**, and a selection drain consumes **one click per key**, in order from key 1 to
key 9, so the highest key clicked wins and a key's further clicks wait for later drains. Hotbar
selection can drain more than once per tick, so its limit is per drain, not per tick: there is no
once-per-tick guard. A full queue refuses the new entry, logs its first refusal once for that queue's
lifetime, and the call returns normally (`Ok(())`), like a lost physical press. Clearing pending
input does not reset that diagnostic. Physical input using these queues has the same limits; ordinary
input below the limits keeps its order and timing.

`key` is a vanilla mapping of the movement, gameplay or inventory category: `key.forward`, `key.left`,
`key.back`, `key.right`, `key.jump`, `key.sneak`, `key.sprint`, `key.attack`, `key.use`, `key.pickItem`,
`key.drop`, `key.inventory`, `key.swapOffhand` and `key.hotbar.1` to `key.hotbar.9`. Both return
`Result<(), MappingError>`:

| Error | When |
|---|---|
| `MappingError::ClickOnly { key }` | `set_mapping_down` on `key.pickItem`; use `click_mapping` instead |
| `MappingError::NoWorld` | no player is in a world |
| `MappingError::Unknown { key }` | no vanilla mapping has that name (`"nope"`) |
| `MappingError::NotGameplay { key }` | a vanilla mapping of another category (`key.screenshot`, `key.fullscreen`, `key.chat`...) |

## What the game does with them

The call only sets the held flag or adds the click. The client tick's key step (`handleKeybinds`) does
the rest:

- Attack, use and pick clicks are taken only with no screen or overlay open. The miss cooldown, busy
  hands and the item checks of `startAttack` and `startUseItem` apply, an entity attack respects the
  item's attack range, and the target is the one under the crosshair.
- An attack on a block sends the break start with its prediction sequence, then the punch packet; an
  attack on nothing sends the punch packet and sets the miss cooldown to 10 ticks (not in creative).
  With the attack mapping not held, the same tick lets go of the break and clears the cooldown, as a
  tap between two ticks does: hold it (`set_mapping_down("key.attack", true)`) for a break that goes on.
- A held use repeats every four ticks and does nothing more while the item is in use; letting go
  releases it (a bow shoots).
- A click on an inventory key acts as its key's press: a hotbar slot is selected on a subsequent tick, and
  drop, the inventory and the hand swap act as they do for the key.
- Sneak, sprint, attack and use toggle when their toggle option is on: `true` flips them and `false`
  does nothing, as `ToggleKeyMapping.setDown`.

With a screen or overlay open, a click is dropped and a press holds nothing. Opening one drops direct
held-state writes, as `KeyMapping.releaseAll` does; set the state again once the screen is closed.

`interaction_key_mapping_triggered` fires at the attack or use as it does for the player's click, and
its cancel and swing work the same. `key_mapping_pre` is not asked: it is about physical key and mouse
button events. `ClientHost::mapping(name)` reads the mapping's logical held state.
`ClientHost::mapping_state(name)` is the same read without the label, for a HUD that reads mappings every
frame: whether the mapping is held, and a hash of the key it is bound to. The hash changes when the binding
does, so read the label with `mapping` only then. It allocates nothing.

`ClientHost::mapping_physical_down(name)` reads the other state: whether the key or mouse button the mapping is
bound to is physically down, following the player's binding and ignoring the toggle option, the conflict
context and any `set_mapping_down` write. A mod that let go of mappings while its own control was in use hands
back what the player still holds with `set_mapping_down(key, mapping_physical_down(key) == Ok(Some(true)))`.
`ClientHost::is_key_down` takes mouse button names (`key.mouse.left`) as well as key names.

The game has no bare swing to add: in 26.3 a swing sends nothing of its own, and the attack mapping
produces the swing and its packet.
