//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `client_options` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ClientOptionsClientHost {
    /// Side: client.
    /// A snapshot of the current input preferences.
    fn input_options(&self) -> ClientInputOptions;
    /// Side: client.
    /// A snapshot of the current graphics preferences.
    fn graphics_options(&self) -> ClientGraphicsOptions;
    /// Side: client.
    /// A snapshot of the current GUI and accessibility preferences. Its
    /// `gui_scale` is only the option, where 0 means automatic; read
    /// [`Self::gui_scale`] for the scale the window applies.
    fn gui_options(&self) -> ClientGuiOptions;
    /// Side: client.
    /// The GUI scale in effect (`Window.getGuiScale`), at least 1; none on a
    /// server or when no window exists. Vanilla's `Window.calculateScale`
    /// resolves it from the GUI Scale option and the window size: the option is
    /// a maximum (0 = automatic, not a guarantee), and with the Unicode font
    /// forced an odd result is raised by one. This read returns the scale the
    /// window applies, never a separate recomputation. Divide
    /// [`Self::window_size`] by this scale, rounding up, for the GUI-scaled
    /// width and height (`Window.getGuiScaledWidth/Height`); draw callbacks
    /// read their GUI dimensions from [`DrawContext::size`].
    fn gui_scale(&self) -> Option<u32>;
    /// Side: client.
    /// The window's width and height in framebuffer pixels (`Window.getWidth`
    /// and `Window.getHeight`), whatever the GUI scale; none on a server or
    /// when no window exists. Readable at any time: `window_resize` reports
    /// this same size, so the two always agree.
    fn window_size(&self) -> Option<[u32; 2]>;
    /// Side: client.
    /// A snapshot of the current chat preferences.
    fn chat_options(&self) -> ClientChatOptions;
    /// Side: client.
    /// A snapshot of the current audio preferences.
    fn audio_options(&self) -> ClientAudioOptions;
    /// Side: client.
    /// Whether a shader pack is currently loaded and active (Iris's
    /// `IrisApi.isShaderPackInUse()`). `false` on a server, when no renderer
    /// exists, or while no shader pack is running.
    fn shader_pack_active(&self) -> bool;
}
