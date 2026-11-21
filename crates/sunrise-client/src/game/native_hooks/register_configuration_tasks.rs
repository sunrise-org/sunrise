use super::Game;
use mod_api::Identifier;
impl Game<'_> {
    pub(in crate::game) fn native_register_configuration_tasks(
        &mut self,
        connection: mod_api::ConfigurationConnection,
        player: u128,
        name: &str,
        tasks: &mut Vec<mod_api::ConfigurationTask>,
    ) {
        self.fire_register_configuration_tasks(connection, player, name, tasks);
    }
    pub(in crate::game) fn native_configuration_task_start(
        &mut self,
        connection: mod_api::ConfigurationConnection,
        task_type: &Identifier,
    ) {
        self.fire_configuration_task_start(connection, task_type);
    }
    pub(in crate::game) fn native_configuration_task_tick(
        &mut self,
        connection: mod_api::ConfigurationConnection,
        task_type: &Identifier,
        finished: &mut bool,
    ) {
        self.fire_configuration_task_tick(connection, task_type, finished);
    }
}
