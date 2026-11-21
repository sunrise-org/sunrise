# Custom consume effects

Implement `Hooks::consume_effect` to handle a consumable's `on_consume_effects`
entry whose type is in your mod's namespace. Only that namespace's enabled
subscriber receives it. The hook runs on the server thread for the consuming
player or mob, including potions, ominous bottles, and witch and trader drinks.
Entries run in the stack's effective consumable list order, interleaved with
built-in effects; stack patches can change or remove the list.

- The event provides the effect kind and its definition payload as a borrowed `Tag`.
- `event.stack` is a read-only snapshot before the stack shrinks, components included.
- `event.user` is the original consuming entity, not necessarily the acting player. The
  handle can be stale before a world is published or if the entity was removed during an
  earlier effect.
- Replacing the stack during an effect does not change what later entries see. The finish
  does not shrink a replacement or generate a remainder from the invalidated use.
- You can apply effects, play sounds, teleport entities and drop items through the host.
- An unclaimed kind does nothing.
- Consumable and death protection payloads are decoded once for item definitions
  and stack component patches. The event borrows the retained payload. Player death
  protection uses this hook; mob death protection does not dispatch it.
- Invalid payloads refuse loading rather than failing during consumption.

To apply an effect without consuming an item, use
[`LivingRef::apply_consume_effect`](consumable-effects.md).

## Food eligibility

`Hooks::can_eat` receives the eating `player` on both sides, with `stack = None`
for cake and candle cake. Read food through `event.player.food(host)` and effects
through `host.living(event.player.entity())`. On the client these are the local
player's copies of server-sent state. Return `Some(true)` to permit eating,
`Some(false)` to refuse it, or `None` to keep `vanilla_can_eat`. Always-eat
eligibility comes from the food component, not a separate event field.
