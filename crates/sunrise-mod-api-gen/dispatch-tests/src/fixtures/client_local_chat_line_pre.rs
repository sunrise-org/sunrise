mod stubs {
    #[derive(Clone, Copy)]
    pub(super) enum LocalChatSource {
        Client,
    }
}

mod crate_root {
    mod text {
        use crate::tests::Text;

        pub(crate) struct TextSnapshot(Text);

        impl TextSnapshot {
            pub(crate) fn new(text: &Text) -> Self {
                Self(text.clone())
            }

            pub(crate) fn restore(self, text: &mut Text) {
                *text = self.0;
            }
        }
    }
}
