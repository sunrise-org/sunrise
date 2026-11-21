//! Operations reached through ConfigurationConnection and ConfigurationTaskContext.
#![allow(unused_variables)]
use crate::*;

/// Server-side configuration capabilities before a player handle exists.
#[sunrise_mod_api_macros::host_area]
pub trait ConfigurationTasksServerHost {
    /// Side: server. Whether the peer negotiated this configuration channel.
    #[access(WorldOnly)]
    fn configuration_has_channel(
        &self,
        connection: ConfigurationConnection,
        channel: &PayloadId,
    ) -> bool;
    /// Side: server. Queue a send on the registered configuration channel of the exact
    /// running task instance, not whichever task is current on the connection. Prefer
    /// [`ConfigurationTaskContext::send_payload`] in start and tick callbacks.
    #[access(WorldOnly)]
    fn configuration_send_payload(
        &mut self,
        instance: ConfigurationTaskRef,
        channel: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<(), ConfigurationTaskError>;
}
