//! Server-thread task queues and progress shared with each connection worker.
use super::config_payloads::ConfigSender;
use crate::raw_payloads::{Endpoint, Transport, connection::Epoch};
use mod_api::{
    __private::RegisteredConfigurationTask, ConfigurationConnection, ConfigurationTaskError,
    PayloadId,
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
struct Entry {
    session: Arc<Session>,
    queue: Vec<Option<RegisteredConfigurationTask>>,
}

#[derive(Default)]
pub struct Tasks {
    sessions: BTreeMap<u64, Entry>,
}
impl Tasks {
    pub fn insert(&mut self, request: &ConfigurationRequest) {
        self.sessions.insert(
            mod_api::__private::configuration_connection_id(request.connection),
            Entry {
                session: request.session.clone(),
                queue: Vec::new(),
            },
        );
    }
    /// Publish a committed registration to the worker.
    pub fn register(
        &mut self,
        connection: ConfigurationConnection,
        tasks: Vec<RegisteredConfigurationTask>,
    ) {
        let Some(entry) = self
            .sessions
            .get_mut(&mod_api::__private::configuration_connection_id(connection))
        else {
            return;
        };
        entry.queue = tasks.into_iter().map(Some).collect();
        entry.session.register(entry.queue.len());
    }
    pub fn get(&self, connection: ConfigurationConnection) -> Option<&Arc<Session>> {
        self.sessions
            .get(&mod_api::__private::configuration_connection_id(connection))
            .map(|entry| &entry.session)
    }
    /// Take the object out of its slot before running or retiring it.
    pub fn take_task(
        &mut self,
        connection: ConfigurationConnection,
        ordinal: usize,
    ) -> Option<RegisteredConfigurationTask> {
        self.sessions
            .get_mut(&mod_api::__private::configuration_connection_id(connection))?
            .queue
            .get_mut(ordinal)?
            .take()
    }
    /// Put back a pending object only while its original session and turn are live.
    /// On refusal the caller still owns the object and retires it unlocked.
    pub fn restore_task(
        &mut self,
        connection: ConfigurationConnection,
        session: &Arc<Session>,
        ordinal: usize,
        task: &mut Option<RegisteredConfigurationTask>,
    ) {
        let Some(entry) = self
            .sessions
            .get_mut(&mod_api::__private::configuration_connection_id(connection))
            .filter(|entry| Arc::ptr_eq(&entry.session, session))
        else {
            return;
        };
        let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.running(ordinal)
            && !session.transport().closed()
            && let Some(slot) = entry.queue.get_mut(ordinal).filter(|slot| slot.is_none())
        {
            *slot = task.take();
        }
    }
    /// Take out a closed session's remaining tasks so the caller can retire them.
    pub fn remove(
        &mut self,
        connection: ConfigurationConnection,
    ) -> Vec<Option<RegisteredConfigurationTask>> {
        self.sessions
            .remove(&mod_api::__private::configuration_connection_id(connection))
            .map_or_else(Vec::new, |entry| entry.queue)
    }
    /// Visit the next session without retaining a borrow across a task callback.
    pub fn next_session(
        &self,
        after: Option<ConfigurationConnection>,
    ) -> Option<(ConfigurationConnection, Arc<Session>)> {
        use std::ops::Bound::{Excluded, Unbounded};
        let lower = after.map_or(Unbounded, |connection| {
            Excluded(mod_api::__private::configuration_connection_id(connection))
        });
        self.sessions
            .range((lower, Unbounded))
            .next()
            .map(|(&id, entry)| {
                (
                    mod_api::__private::configuration_connection(id),
                    entry.session.clone(),
                )
            })
    }
}
#[derive(Default)]
struct State {
    task_count: Option<usize>,
    at: usize,
    started: bool,
    finished: bool,
    failure: Option<String>,
    closed: bool,
}
impl State {
    fn running(&self, ordinal: usize) -> bool {
        self.at == ordinal
            && self.started
            && !self.finished
            && !self.closed
            && self.failure.is_none()
    }
}
pub struct Turn {
    pub ordinal: usize,
    pub start: bool,
}

/// A completion held until commit, bound to exactly one session and queue entry.
pub struct ConfigurationCompletion {
    session: Arc<Session>,
    ordinal: usize,
    ingress: Option<Arc<Epoch>>,
}
impl ConfigurationCompletion {
    pub fn publish(self) {
        let mut state = self.session.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.running(self.ordinal)
            && !self.session.transport().closed()
            && self.ingress.as_ref().is_none_or(|epoch| epoch.active())
        {
            state.finished = true;
        }
    }
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
    fn register(&self, task_count: usize) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .task_count = Some(task_count);
    }
    /// Advance past a published completion, returning its ordinal for retirement
    /// even when it was the final task. Then take the next eligible turn, if any.
    pub fn turn(&self) -> (Option<usize>, Option<Turn>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(task_count) = state.task_count else {
            return (None, None);
        };
        if state.closed || state.failure.is_some() || self.transport().closed() {
            return (None, None);
        }
        let completed = state.finished.then_some(state.at);
        if completed.is_some() {
            state.at += 1;
            state.started = false;
            state.finished = false;
        }
        let turn = (state.at < task_count).then(|| {
            let start = !state.started;
            state.started = true;
            Turn {
                ordinal: state.at,
                start,
            }
        });
        (completed, turn)
    }
    pub fn finished(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .finished
    }
    /// Resolve an issued ordinal without mutating or failing any other turn.
    pub fn require_running(&self, ordinal: usize) -> Result<(), ConfigurationTaskError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || state.failure.is_some() || self.transport().closed() {
            return Err(ConfigurationTaskError::Disconnected);
        }
        if !state.running(ordinal) {
            return Err(ConfigurationTaskError::StaleTask);
        }
        Ok(())
    }
    /// Validate before staging; a delayed ingress also needs its epoch alive at publication.
    pub fn prepare_finish(
        self: &Arc<Self>,
        ordinal: usize,
        ingress: Option<Arc<Epoch>>,
    ) -> Result<ConfigurationCompletion, ConfigurationTaskError> {
        self.require_running(ordinal)?;
        Ok(ConfigurationCompletion {
            session: self.clone(),
            ordinal,
            ingress,
        })
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
    pub fn transport(&self) -> &Arc<Transport> {
        self.sender.payload_transport()
    }
    pub fn result(&self) -> Option<Result<(), String>> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &state.failure {
            return Some(Err(error.clone()));
        }
        (state.task_count == Some(state.at)).then_some(Ok(()))
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
    fn completion_only_takes_effect_on_publication_for_its_own_turn() {
        use crate::lan::config_payloads::ConfigIo;
        use sunrise_protocol::connection::{Connection, Pipe};

        let (_peer, server) = Pipe::pair();
        let mut server = Connection::local(server);
        let transport = Transport::empty();
        let (events, _receiver) = crate::lan::event_channel_with_capacity(1 << 20);
        let mut io = ConfigIo::new(&mut server, transport.clone(), events).unwrap();
        let session = Session::new(io.task_sender(transport).unwrap());
        session.register(2);
        let (completed, turn) = session.turn();
        assert_eq!(completed, None);
        let turn = turn.unwrap();
        assert!(turn.start);
        assert_eq!(turn.ordinal, 0);
        let completion = session.prepare_finish(turn.ordinal, None).unwrap();
        let delayed = session.prepare_finish(turn.ordinal, None).unwrap();
        assert!(!session.finished());
        let (completed, tick) = session.turn();
        assert_eq!(completed, None);
        let tick = tick.unwrap();
        assert!(!tick.start);
        assert_eq!(tick.ordinal, 0);

        completion.publish();
        assert!(session.finished());
        let (completed, next) = session.turn();
        assert_eq!(completed, Some(0));
        let next = next.unwrap();
        assert!(next.start);
        assert_eq!(next.ordinal, 1);
        delayed.publish();
        assert!(!session.finished());
        assert!(matches!(
            session.prepare_finish(turn.ordinal, None),
            Err(ConfigurationTaskError::StaleTask)
        ));

        session
            .prepare_finish(next.ordinal, None)
            .unwrap()
            .publish();
        let (completed, turn) = session.turn();
        assert_eq!(completed, Some(1));
        assert!(turn.is_none());
        assert_eq!(session.result(), Some(Ok(())));
    }
}
