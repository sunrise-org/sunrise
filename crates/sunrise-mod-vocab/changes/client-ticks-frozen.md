# `EntityRef::ticks_frozen` reads on a client

`EntityRef::ticks_frozen` (`Entity.getTicksFrozen`) now reads on a client too: the synced `DATA_TICKS_FROZEN` of the local player and of tracked mobs, so a HUD mod that draws its own hearts can pick the frozen ones (#5693). The method's signature is unchanged.

A client that read `None` now reads the synced count; nothing needs to change.
