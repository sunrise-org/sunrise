mod stubs {
    #[derive(Clone, Copy)]
    pub(super) struct EntityUuid(pub(super) u128);

    #[derive(Clone, Copy)]
    pub(super) enum PlayerLayerView {
        World,
    }

    #[derive(Clone, Copy)]
    pub(super) enum PlayerLayer {
        Cape,
    }

    #[derive(Clone, Copy)]
    pub(super) struct PlayerLayerSelection<'a>(std::marker::PhantomData<&'a ()>);

    impl PlayerLayerSelection<'_> {
        pub(super) fn __stage(&self, _: &'static str) -> Self {
            *self
        }
    }
}
