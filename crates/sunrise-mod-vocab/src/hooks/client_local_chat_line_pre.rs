//! The local chat insertion hook, read by the mod API generator.

pub trait Hooks {
    /// A line created locally is about to enter the client's chat HUD and history
    /// (`ChatComponent.addMessage`), after the Chat option allows its `source`.
    /// This includes game notices and lines from `ClientHost::chat_line`, even
    /// without a connection. The source describes visibility, not a sender or signature.
    ///
    /// Use `event.message_mut()` to edit the styled line or cancel to skip it,
    /// including logging and accounting. If the original cannot cross the text
    /// boundary (for example, it has a trusted file link), this hook is skipped.
    /// If the replacement is invalid, the original is
    /// kept unless cancelled. A failing listener's edits (including cancellation)
    /// are discarded, and later listeners still run in order. An insertion
    /// inside a transaction is published only on commit. The shared host cannot
    /// insert another line.
    ///
    /// Received messages use `client_chat_received` instead. Redraw, rewrap and
    /// replay of chat history do not fire this hook. Client-side.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        read_only = "source",
        isolate = "listener",
        neoforge = "none (ClientChatReceivedEvent is for received chat)",
        cancellable = true
    )]
    fn client_local_chat_line_pre(source: LocalChatSource, message: &mut Text, cancel: &mut bool);
}
