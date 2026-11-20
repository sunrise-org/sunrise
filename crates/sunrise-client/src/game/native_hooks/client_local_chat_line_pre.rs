use super::super::Game;

impl Game<'_> {
    pub(in crate::game) fn native_client_local_chat_line_pre(
        &self,
        call: mod_api::__private::ClientLocalChatLinePreCall<'_>,
    ) {
        self.fire_client_local_chat_line_pre(call);
    }
}
