//! Configuration tasks and capabilities valid before the player joins.
use crate::{Identifier, PayloadBytes, PayloadId, RegisterConfigurationTasksEvent, ServerHost};
use std::{collections::BTreeSet, fmt};

// no wire: tasks are owned objects with server-thread callbacks and local state.
/// One task instance owned by a configuring connection. Register the instance with
/// [`RegisterConfigurationTasksEvent::register`]. The server runs it in queue order,
/// on the server thread without holding the connection's task lock, and keeps the
/// same instance while it waits.
pub trait ConfigurationTask: Send + 'static {
    /// The namespaced task type, read once on registration. It must be nonempty and
    /// unique among all tasks registered for this connection.
    fn task_type(&self) -> &Identifier;

    /// Start this task exactly once. Call [`ConfigurationTaskContext::finish`] or
    /// [`ConfigurationTaskContext::fail`], or leave it pending for a tick or a reply.
    /// A finish request publishes only after the outermost transaction commits and
    /// publishes its deferred sends.
    fn start(&mut self, host: &mut dyn ServerHost, context: &mut ConfigurationTaskContext<'_>);

    /// Poll a pending task, at most once per server tick after its start. By default
    /// the task just waits. As in [`Self::start`], use the context to send, finish or
    /// fail. A failure or panic rolls back the callback and fails admission.
    fn tick(&mut self, _host: &mut dyn ServerHost, _context: &mut ConfigurationTaskContext<'_>) {}
}

// no wire: local registration diagnostics, not payload data.
/// An invalid submission to the connection's configuration task queue.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigurationTaskRegistrationError {
    /// The task type has an empty namespace or path.
    EmptyType,
    /// Another task already declared this type for this connection.
    DuplicateType(Identifier),
}
impl fmt::Display for ConfigurationTaskRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyType => {
                f.write_str("Configuration task type has an empty namespace or path")
            }
            Self::DuplicateType(id) => write!(f, "Duplicate configuration task type: {id}"),
        }
    }
}
impl std::error::Error for ConfigurationTaskRegistrationError {}

impl RegisterConfigurationTasksEvent<'_> {
    /// Append a task to this connection's queue. An invalid or duplicate type refuses
    /// registration, aborting the registration transaction and admission even if the
    /// listener ignores this result. The first refusal supplies the diagnostic.
    pub fn register(
        &mut self,
        task: impl ConfigurationTask,
    ) -> Result<(), ConfigurationTaskRegistrationError> {
        let id = task.task_type();
        let refusal = if id.namespace().is_empty() || id.path().is_empty() {
            ConfigurationTaskRegistrationError::EmptyType
        } else if !self.registration.types.insert(id.clone()) {
            ConfigurationTaskRegistrationError::DuplicateType(id.clone())
        } else {
            self.registration.tasks.push(RegisteredConfigurationTask {
                task: Box::new(task),
                owner: crate::content::crash::current_native_mod(),
            });
            return Ok(());
        };
        self.registration
            .refusal
            .get_or_insert_with(|| refusal.clone());
        Err(refusal)
    }
}

/// Engine staging for one registration transaction.
// no wire: owns pending server-thread tasks.
// hidden: the server stages registrations here; listeners use the scoped event.
// exhaustive: the server starts empty staging with Default; the queue and refusal fields are private.
#[doc(hidden)]
#[derive(Default)]
pub struct ConfigurationTaskRegistration {
    tasks: Vec<RegisteredConfigurationTask>,
    types: BTreeSet<Identifier>,
    refusal: Option<ConfigurationTaskRegistrationError>,
}
impl ConfigurationTaskRegistration {
    /// The first refusal, if any, even if the listener ignored the error.
    pub fn refusal(&self) -> Option<&ConfigurationTaskRegistrationError> {
        self.refusal.as_ref()
    }

    /// Move the ordered queue to the server after registration commits.
    pub fn into_tasks(self) -> Vec<RegisteredConfigurationTask> {
        self.tasks
    }
}

/// Engine queue entry with the callback's mod scope resolved at registration.
// no wire: an owned task and its process-local mod identity.
// hidden: the server owns these queue entries; listeners register through the scoped event.
#[doc(hidden)]
#[non_exhaustive]
pub struct RegisteredConfigurationTask {
    /// The connection-owned instance.
    pub task: Box<dyn ConfigurationTask>,
    /// The registering mod, for Host calls that depend on the caller's identity.
    pub owner: Option<&'static str>,
}

/// One configuring connection, available while registering its tasks.
// no wire: a local capability for this connection, not payload data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConfigurationConnection {
    id: u64,
}
impl ConfigurationConnection {
    /// Whether this peer negotiated the registered configuration payload channel.
    pub fn has_channel(self, host: &dyn ServerHost, channel: &PayloadId) -> bool {
        host.configuration_has_channel(self, channel)
    }
}

/// The exact running task of one configuring connection. For delayed completion,
/// keep the instance from the original challenge in server-local state. Never
/// substitute the task that happens to be running when the reply arrives.
// no wire: a process-local capability, never supplied by a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConfigurationTaskRef {
    connection: ConfigurationConnection,
    ordinal: usize,
}

/// The active task during one server-thread start or tick callback.
// no wire: borrows the callback's outcome.
#[non_exhaustive]
pub struct ConfigurationTaskContext<'a> {
    instance: ConfigurationTaskRef,
    outcome: &'a mut ConfigurationTaskOutcome,
}
impl ConfigurationTaskContext<'_> {
    /// The exact task instance, which can outlive this callback. When sending a
    /// challenge, save this value for the matching reply handler.
    pub fn instance(&self) -> ConfigurationTaskRef {
        self.instance
    }
    /// Whether this task's peer negotiated the registered configuration channel.
    pub fn has_channel(&self, host: &dyn ServerHost, channel: &PayloadId) -> bool {
        self.instance.connection.has_channel(host, channel)
    }
    /// Queue a payload in the current transaction on this task's registered channel.
    /// A transport refusal fails the task.
    pub fn send_payload(
        &self,
        host: &mut dyn ServerHost,
        channel: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<(), ConfigurationTaskError> {
        host.configuration_send_payload(self.instance, channel, bytes)
    }
    /// Request completion after the outermost transaction commits and publishes its
    /// deferred sends. Repeated requests coalesce; failure, panic or disconnect prevents it.
    pub fn finish(&mut self) {
        if matches!(self.outcome, ConfigurationTaskOutcome::Pending) {
            *self.outcome = ConfigurationTaskOutcome::Finish;
        }
    }
    /// Fail configuration for this peer and discard the callback's changes and any finish
    /// request. The first explicit failure supplies the diagnostic.
    pub fn fail(&mut self, reason: &str) {
        if !matches!(self.outcome, ConfigurationTaskOutcome::Fail(_)) {
            *self.outcome = ConfigurationTaskOutcome::Fail(reason.into());
        }
    }
}

/// A task operation was refused.
// no wire: local task and transport diagnostics, not payload data.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigurationTaskError {
    /// The connection or its configuration epoch has ended.
    Disconnected,
    /// The instance is not the currently running task.
    StaleTask,
    /// The instance belongs to a different physical connection than the receiving peer.
    WrongConnection,
    /// Completion needs a server main-thread configuration handler with an open transaction.
    WrongContext,
    /// The registered payload transport refused the send.
    Payload(crate::PayloadSendError),
}
impl crate::NoBackend for ConfigurationTaskError {
    fn no_backend() -> Self {
        Self::Disconnected
    }
}

// hidden: the engine reads the callback outcome when closing its transaction.
// no wire: the engine's local decision for one callback.
// exhaustive: matched only by the engine.
#[doc(hidden)]
pub enum ConfigurationTaskOutcome {
    Pending,
    Finish,
    Fail(String),
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
// hidden: only the engine issues an instance for a queue entry.
#[doc(hidden)]
pub fn configuration_task_ref(
    connection: ConfigurationConnection,
    ordinal: usize,
) -> ConfigurationTaskRef {
    ConfigurationTaskRef {
        connection,
        ordinal,
    }
}
// hidden: the engine resolves the instance against its own queue.
#[doc(hidden)]
pub fn configuration_task_parts(
    instance: ConfigurationTaskRef,
) -> (ConfigurationConnection, usize) {
    (instance.connection, instance.ordinal)
}
// hidden: the engine lends the callback its outcome slot.
#[doc(hidden)]
pub fn configuration_task_context(
    instance: ConfigurationTaskRef,
    outcome: &mut ConfigurationTaskOutcome,
) -> ConfigurationTaskContext<'_> {
    ConfigurationTaskContext { instance, outcome }
}
