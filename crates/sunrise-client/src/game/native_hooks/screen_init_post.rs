use super::*;
use mod_api::client::widgets::ScreenWidgetBuild;

impl Game<'_> {
    pub(in crate::game) fn has_screen_init_post(&self) -> bool {
        self.options.execution_role.is_client()
            && self.lan_acting.is_none()
            && mod_api::__private::listened(mod_api::HookId::ScreenInitPost)
    }

    pub(in crate::game) fn has_screen_init_hooks(&self) -> bool {
        self.has_screen_init_post()
            || (self.options.execution_role.is_client()
                && self.lan_acting.is_none()
                && mod_api::__private::listened(mod_api::HookId::ScreenInitPre))
    }

    pub(in crate::game) fn native_screen_init_pre(
        &self,
        screen: &ScreenView<'_>,
        canceled: &mut bool,
    ) {
        self.fire_screen_init_pre(screen, canceled);
    }

    /// The stock screen has built its widgets and established initial focus.
    pub(in crate::game) fn native_screen_init_post(
        &self,
        screen: &ScreenView<'_>,
        widgets: &mut ScreenWidgetBuild,
    ) {
        self.fire_screen_init_post(screen, widgets);
    }
}
