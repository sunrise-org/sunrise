// Dispatch-only values; real handles/shapes and Wire are covered by the API/Game tests.
mod stubs {
    #[derive(Clone, Copy)]
    pub(super) struct LiveWorldRef;
    pub(super) struct LiveWorldPlacementCell;
}
