//! Main-thread configuration task state shared with one connection worker.
use super::config_payloads::ConfigSender;
use crate::raw_payloads::{Endpoint, Transport};
use mod_api::{
    ConfigurationConnection, ConfigurationTask, ConfigurationTaskError, Identifier, PayloadId,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use sunrise_protocol::payload_negotiation::Phase;

pub struct ConfigurationRequest {
    pub connection: ConfigurationConnection,
    pub player: u128,
    pub name: String,
    pub session: Arc<Session>,
}
/// Process-unique identities keep a capability stale even after a server restarts.
pub(super) fn next_connection() -> Option<ConfigurationConnection> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .ok()
        .map(mod_api::__private::configuration_connection)
}
#[derive(Default)]
pub struct Tasks {
    sessions: BTreeMap<u64, Arc<Session>>,
}
impl Tasks {
    pub fn insert(&mut self, request: &ConfigurationRequest) {
        self.sessions.insert(
            mod_api::__private::configuration_connection_id(request.connection),
            request.session.clone(),
        );
    }
    pub fn get(&self, connection: ConfigurationConnection) -> Option<&Arc<Session>> {
        self.sessions
            .get(&mod_api::__private::configuration_connection_id(connection))
    }
    /// Visit the next session without retaining a borrow across a hook callback.
    pub fn next_running(
        &mut self,
        after: Option<ConfigurationConnection>,
    ) -> Option<(ConfigurationConnection, Arc<Session>)> {
        use std::ops::Bound::{Excluded, Unbounded};
        if after.is_none() {
            self.sessions.retain(|_, session| !session.closed());
        }
        let lower = after.map_or(Unbounded, |connection| {
            Excluded(mod_api::__private::configuration_connection_id(connection))
        });
        self.sessions
            .range((lower, Unbounded))
            .next()
            .map(|(&id, session)| {
                (
                    mod_api::__private::configuration_connection(id),
                    session.clone(),
                )
            })
    }
}
#[derive(Default)]
struct State {
    tasks: Vec<Identifier>,
    at: usize,
    registered: bool,
    started: bool,
    finished: bool,
    failure: Option<String>,
    closed: bool,
}
pub struct Turn {
    pub task_type: Identifier,
    pub start: bool,
}
pub struct Session {
    state: Mutex<State>,
    sender: ConfigSender,
}
impl Session {
    pub fn new(sender: ConfigSender) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            sender,
        })
    }
    pub fn register(&self, tasks: Vec<ConfigurationTask>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut ids = std::collections::BTreeSet::new();
        if tasks.iter().any(|task| {
            task.id.namespace().is_empty()
                || task.id.path().is_empty()
                || !ids.insert(task.id.clone())
        }) {
            state.failure = Some("Invalid or duplicate configuration task type".into());
            return;
        }
        state.tasks = tasks.into_iter().map(|task| task.id).collect();
        state.registered = true;
    }
    pub fn turn(&self) -> Option<Turn> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || state.failure.is_some() || !state.registered {
            return None;
        }
        if state.finished {
            state.at += 1;
            state.started = false;
            state.finished = false;
        }
        let task_type = state.tasks.get(state.at)?.clone();
        let start = !state.started;
        state.started = true;
        Some(Turn { task_type, start })
    }
    pub fn finished(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .finished
    }
    pub fn active(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.registered
            && state.started
            && !state.finished
            && !state.closed
            && state.failure.is_none()
    }
    pub fn finish(&self, task_type: &Identifier) -> Result<(), ConfigurationTaskError> {
        if self.closed() {
            return Err(ConfigurationTaskError::Disconnected);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || state.failure.is_some() {
            return Err(ConfigurationTaskError::Disconnected);
        }
        if !state.started || state.finished || state.tasks.get(state.at) != Some(task_type) {
            state.failure = Some("Finished a configuration task with the wrong type".into());
            return Err(ConfigurationTaskError::WrongType);
        }
        state.finished = true;
        Ok(())
    }
    pub fn fail(&self, reason: impl Into<String>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.failure.get_or_insert_with(|| reason.into());
    }
    pub fn has_channel(&self, channel: &PayloadId) -> bool {
        !self.closed()
            && self
                .transport()
                .epoch()
                .is_some_and(|epoch| epoch.peer.can_send(Phase::Configuration, channel).is_ok())
    }
    pub fn endpoint(&self) -> Endpoint {
        Endpoint::configuration(self.sender.clone())
    }
    fn transport(&self) -> &Arc<Transport> {
        self.sender.payload_transport()
    }
    pub fn result(&self) -> Option<Result<(), String>> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &state.failure {
            return Some(Err(error.clone()));
        }
        (state.registered && state.at == state.tasks.len()).then_some(Ok(()))
    }
    pub fn close(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
    }
    pub fn closed(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed || self.transport().closed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finishing_twice_before_the_next_turn_refuses_and_fails_configuration() {
        use crate::lan::config_payloads::ConfigIo;
        use sunrise_protocol::connection::{Connection, Pipe};

        let (_peer, server) = Pipe::pair();
        let mut server = Connection::local(server);
        let transport = Transport::empty();
        let (events, _receiver) = crate::lan::event_channel_with_capacity(1 << 20);
        let mut io = ConfigIo::new(&mut server, transport.clone(), events).unwrap();
        let session = Session::new(io.task_sender(transport).unwrap());
        let task_type = Identifier::parse("task_fixture:first").unwrap();
        session.register(vec![ConfigurationTask::new(task_type.clone())]);
        assert!(session.turn().unwrap().start);
        assert_eq!(session.finish(&task_type), Ok(()));
        // No intervening turn: completion itself must consume the right to finish.
        assert_eq!(
            session.finish(&task_type),
            Err(ConfigurationTaskError::WrongType)
        );
        assert!(matches!(session.result(), Some(Err(_))));
        assert!(session.turn().is_none());
    }
}
