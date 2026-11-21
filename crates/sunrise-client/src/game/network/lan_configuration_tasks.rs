//! Dispatch each configuring connection on the server's ordinary tick.
use super::{Game, lan::LanHost};
use mod_api::{
    ConfigurationConnection, ConfigurationTaskError, Identifier, PayloadBytes, PayloadId,
};
use std::{io, sync::Arc};
use sunrise_client::lan::{Lan, NetConfig, configuration_tasks::Session, data::SyncData};

pub(super) fn start(sync: Arc<SyncData>, net: NetConfig) -> io::Result<Lan> {
    Lan::start_with_configuration_tasks(
        sync,
        net,
        mod_api::__private::listened(mod_api::HookId::RegisterConfigurationTasks),
    )
}
impl Game<'_> {
    fn configuration_callback(&mut self, session: &Session, callback: impl FnOnce(&mut Self)) {
        let transaction = self.transaction_open();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(self)));
        self.transaction_close(
            &transaction,
            result.is_ok() && !matches!(session.result(), Some(Err(_))),
        );
        if result.is_err() {
            session.fail("Configuration task callback panicked");
        }
    }
    pub(super) fn take_lan_for_packet_tick(&mut self) -> Option<LanHost> {
        if self.level_pass.first {
            let requests: Vec<_> = self
                .lan
                .as_ref()?
                .hub
                .lan
                .configuration_requests()
                .collect();
            for request in requests {
                if request.session.closed() {
                    continue;
                }
                self.lan
                    .as_mut()?
                    .hub
                    .lan
                    .configuration_tasks
                    .insert(&request);
                let mut tasks = Vec::new();
                self.configuration_callback(&request.session, |game| {
                    game.native_register_configuration_tasks(
                        request.connection,
                        request.player,
                        &request.name,
                        &mut tasks,
                    );
                });
                request.session.register(tasks);
            }
            let mut after = None;
            while let Some((connection, session)) = self
                .lan
                .as_mut()?
                .hub
                .lan
                .configuration_tasks
                .next_running(after)
            {
                after = Some(connection);
                while let Some(turn) = session.turn() {
                    self.configuration_callback(&session, |game| {
                        if turn.start {
                            game.native_configuration_task_start(connection, &turn.task_type);
                        } else {
                            let mut finished = false;
                            game.native_configuration_task_tick(
                                connection,
                                &turn.task_type,
                                &mut finished,
                            );
                            // A listener may also have called `finish` itself: one completion.
                            if finished && !session.finished() {
                                let _ = session.finish(&turn.task_type);
                            }
                        }
                    });
                    if !turn.start || !session.finished() {
                        break;
                    }
                }
            }
        }
        self.lan.take()
    }
    fn configuration_session(&self, connection: ConfigurationConnection) -> Option<&Arc<Session>> {
        self.lan
            .as_ref()?
            .hub
            .lan
            .configuration_tasks
            .get(connection)
    }
    pub(in crate::game) fn configuration_has_channel(
        &self,
        connection: ConfigurationConnection,
        channel: &PayloadId,
    ) -> bool {
        self.configuration_session(connection)
            .is_some_and(|session| session.has_channel(channel))
    }
    pub(in crate::game) fn configuration_send_payload(
        &mut self,
        connection: ConfigurationConnection,
        channel: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<(), ConfigurationTaskError> {
        let session = self
            .configuration_session(connection)
            .cloned()
            .ok_or(ConfigurationTaskError::Disconnected)?;
        if !session.active() {
            session.fail("Configuration payload sent without an active task");
            return Err(ConfigurationTaskError::NotRunning);
        }
        self.queue_current_payload(session.endpoint(), channel, bytes)
            .map(|_| ())
            .map_err(|error| {
                session.fail(format!("Configuration payload refused: {error:?}"));
                ConfigurationTaskError::Payload(error)
            })
    }
    pub(in crate::game) fn configuration_finish(
        &mut self,
        connection: ConfigurationConnection,
        task_type: &Identifier,
    ) -> Result<(), ConfigurationTaskError> {
        self.configuration_session(connection)
            .ok_or(ConfigurationTaskError::Disconnected)?
            .finish(task_type)
    }
    pub(in crate::game) fn configuration_fail(
        &mut self,
        connection: ConfigurationConnection,
        reason: &str,
    ) {
        if let Some(session) = self.configuration_session(connection) {
            session.fail(reason);
        }
    }
}
