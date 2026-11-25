//! The command registries the game keeps in step with what the world's content and data
//! packs define, outside the one-time build in `chat.rs`.
use super::Game;

impl Game<'_> {
    /// Active enchantment registry snapshot for command codecs and completions.
    /// Replace both ID views; keep the existing registry tags intact.
    pub(super) fn sync_enchantment_registries(&mut self) {
        let ids: Vec<String> = self
            .active_enchantment_definitions()
            .list
            .iter()
            .map(|entry| entry.id.to_string())
            .collect();
        self.ui.registries.set_enchantment_ids(ids);
    }
}
