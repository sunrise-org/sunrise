//! A configuration task's tick callback.
pub trait Hooks {
    /// Check this running task once per server tick. Set finished when its check succeeds.
    /// connection.finish(host, task_type) verifies the active type. An error or
    /// panic fails this connecting client with the configuration-error reason.
    #[hook(side = "server", contract = "collect", out = "finished")]
    fn configuration_task_tick();
}
