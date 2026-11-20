mod stubs {
    pub(super) struct ScreenWidgetBuild(usize);

    pub(super) struct ScreenWidgets<'a> {
        _build: &'a mut ScreenWidgetBuild,
    }

    impl ScreenWidgetBuild {
        pub(super) fn __checkpoint(&self) -> usize {
            self.0
        }

        pub(super) fn __rollback(&mut self, checkpoint: usize) {
            self.0 = checkpoint;
        }

        pub(super) fn __edit(&mut self, _: &'static str) -> ScreenWidgets<'_> {
            ScreenWidgets { _build: self }
        }
    }
}
