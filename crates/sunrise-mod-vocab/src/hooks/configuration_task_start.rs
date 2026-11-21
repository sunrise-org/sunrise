//! A configuration task's start callback.
pub trait Hooks {
    /// Start this task once. It may send payloads, finish synchronously, or wait for a reply.
    /// connection.finish(host, task_type) verifies the active type. An error or
    /// panic fails this connecting client with the configuration-error reason.
    #[hook(side = "server", contract = "listen")]
    fn configuration_task_start();
}
