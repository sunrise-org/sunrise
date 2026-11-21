//! The mid-setupAnim humanoid animation moment.

pub trait Hooks {
    /// Once per actual `HumanoidModel.setupAnim` invocation, after head/walk/riding
    /// setup and before arm priority/posing. `parts` contains finite local positions
    /// (texels) and rotations (radians); its setters refuse non-finite components
    /// while retaining their prior values. Later vanilla ITEM, attack, crouch, bob
    /// and swim animation still apply. Armor stands then replace rotations with
    /// their saved pose, preserving position edits. Listeners see earlier accepted writes.
    /// `humanoid` contains this invocation's held stacks and use state. Time is
    /// captured for its view; `game_time` is absent outside a level, never wall time.
    /// The Host is read-only. No frame deduplication: layers that actually animate
    /// again get another invocation; copying parent poses does not dispatch.
    #[hook(
        side = "client",
        contract = "listen",
        version = 2,
        neoforge = "none (vanilla HumanoidModel.setupAnim; NeoForge has no mid-setupAnim event)"
    )]
    fn humanoid_setup_anim(
        entity_id: u32,
        entity_type: &str,
        humanoid: &HumanoidRenderState<'_>,
        partial_tick: f32,
        game_time: Option<i64>,
        parts: &mut HumanoidParts,
    );
}
