//! Source categories for locally inserted chat.

/// The engine-assigned visibility category of a local chat line.
/// It does not identify a sender or attest to a signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the two system-message visibility categories
pub enum LocalChatSource {
    /// `GuiMessageSource.SYSTEM_CLIENT`: not hidden by the Chat option.
    Client,
    /// `GuiMessageSource.SYSTEM_SERVER`: hidden when the Chat option is Hidden,
    /// even for a notice generated locally.
    Server,
}
