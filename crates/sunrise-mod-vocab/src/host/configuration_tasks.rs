//! Operations on a joining connection, reached through ConfigurationConnection.
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
    /// Side: server. Send on the current configuration task's registered channel.
    #[access(WorldOnly)]
    fn configuration_send_payload(
        &mut self,
        connection: ConfigurationConnection,
        channel: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<(), ConfigurationTaskError>;
    /// Side: server. Finish only the current task's exact type.
    #[access(WorldOnly)]
    fn configuration_finish(
        &mut self,
        connection: ConfigurationConnection,
        task_type: &Identifier,
    ) -> Result<(), ConfigurationTaskError>;
    /// Side: server. Fail only this connection's configuration.
    #[access(WorldOnly)]
    fn configuration_fail(&mut self, connection: ConfigurationConnection, reason: &str);
}
