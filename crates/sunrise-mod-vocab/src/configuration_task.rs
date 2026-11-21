//! Configuration tasks and a connection capability valid before the player joins.
use crate::{Identifier, PayloadBytes, PayloadId, ServerHost};

/// A task type queued once for this joining connection.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct ConfigurationTask {
    /// Namespaced type passed to the task start and tick callbacks.
    pub id: Identifier,
}
impl ConfigurationTask {
    /// Queue an instance of this task type.
    pub fn new(id: Identifier) -> Self {
        Self { id }
    }
}

/// One configuring connection. A stale capability cannot finish another connection's task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct ConfigurationConnection {
    id: u64,
}
impl ConfigurationConnection {
    /// Whether this peer negotiated the registered configuration payload channel.
    pub fn has_channel(self, host: &dyn ServerHost, channel: &PayloadId) -> bool {
        host.configuration_has_channel(self, channel)
    }
    /// Send through the registered configuration transport. A refusal fails this task.
    pub fn send_payload(
        self,
        host: &mut dyn ServerHost,
        channel: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<(), ConfigurationTaskError> {
        host.configuration_send_payload(self, channel, bytes)
    }
    /// Finish the current task. A different task type is refused and fails configuration.
    pub fn finish(
        self,
        host: &mut dyn ServerHost,
        task_type: &Identifier,
    ) -> Result<(), ConfigurationTaskError> {
        host.configuration_finish(self, task_type)
    }
    /// Fail this connection's task with a diagnostic for its disconnect reason.
    pub fn fail(self, host: &mut dyn ServerHost, reason: &str) {
        host.configuration_fail(self, reason)
    }
}

/// A task operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum ConfigurationTaskError {
    /// The connection has left configuration.
    Disconnected,
    /// The connection has no active task to send from.
    NotRunning,
    /// Finish named a type other than the running task.
    WrongType,
    /// The registered payload transport refused the send.
    Payload(crate::PayloadSendError),
}
impl crate::NoBackend for ConfigurationTaskError {
    fn no_backend() -> Self {
        Self::Disconnected
    }
}

// hidden: the engine makes capabilities after allocating a connection identity.
#[doc(hidden)]
pub fn configuration_connection(id: u64) -> ConfigurationConnection {
    ConfigurationConnection { id }
}
// hidden: the engine resolves and validates a capability before every operation.
#[doc(hidden)]
pub fn configuration_connection_id(connection: ConfigurationConnection) -> u64 {
    connection.id
}
