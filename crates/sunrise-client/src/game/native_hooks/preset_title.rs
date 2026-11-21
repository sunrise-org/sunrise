use super::*;

impl Game<'_> {
    pub(in crate::game) fn show_title_with_registry(
        &mut self,
        mut title: sunrise_client::title::Title,
    ) {
        title.set_additional_data_packs(self.pack.additional_data.clone());
        title.set_preset_data_source(self.pack.data_changes.clone());
        if !self.screen_hooks_heard() {
            self.screen_state.build.reopened();
            self.title = Some(title);
            self.build_screen();
            return;
        }
        let answer = self.title_change(Some(title.mode.id()));
        self.screen_state.build.reopened();
        self.title = Some(title);
        match self.screen_decision(answer) {
            ScreenDecision::ModScreen(mod_id) => self.apply_screen(Screen::Mod, Some(mod_id)),
            ScreenDecision::Vanilla(target) => self.open_screen_target(target),
            // With no world nothing else can show: a `Cancel` doesn't keep it shut.
            ScreenDecision::Keep | ScreenDecision::Open => self.build_screen(),
        }
        self.screen_state.observed = self.observed_now();
        self.shown_init_post();
    }
}
