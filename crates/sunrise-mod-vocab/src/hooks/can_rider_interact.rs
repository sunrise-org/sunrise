//! Entity-part control of the client's same-vehicle pick restriction.

pub trait Hooks {
    /// Whether this candidate counts normally in the client's ordinary entity
    /// pick despite sharing the camera's root vehicle. Mirrors NeoForge's
    /// `IEntityExtension.canRiderInteract`, false by default. Only parts attached
    /// to the candidate's type are asked, in dependency order; `allowed` starts
    /// false and the last write wins. Asked only at the same-vehicle branch of
    /// `ProjectileUtil.getEntityHitResult`, after a non-inside ray intersection
    /// qualifies against the running nearest distance. True lets this candidate
    /// update that distance normally. Inside-box selection does not ask it.
    /// Both the tick and frame pick use this rule. Attack-range component rays
    /// have no same-vehicle restriction and do not ask it. No server reach or
    /// interaction checks change.
    #[hook(
        side = "client",
        contract = "collect",
        out = "allowed",
        scope = "entity",
        key = "entity",
        neoforge = "IEntityExtension.canRiderInteract"
    )]
    fn can_rider_interact(entity: EntityRef, allowed: &mut bool);
}
