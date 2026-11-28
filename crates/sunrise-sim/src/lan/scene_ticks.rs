use super::{Lan, NetEvent, configuration_tasks::ConfigurationRequest};
use crate::raw_payloads::ingress::OwnedReceipt;
use std::{
    cell::RefCell,
    collections::BTreeSet,
    time::{Duration, Instant},
};
use sunrise_protocol::packets::play::Serverbound;

pub(super) fn enabled() -> bool {
    std::env::var("SUNRISE_TEST_SCENE_TICKS").is_ok_and(|value| value == "1")
}

#[derive(Default)]
pub(super) struct Staged {
    pub(super) config: RefCell<Vec<ConfigurationRequest>>,
    pub(super) events: RefCell<Vec<OwnedReceipt<NetEvent>>>,
}

impl Lan {
    pub fn scene_wait_configuration(&self, name: &str) -> Result<(), String> {
        let staged = self.scene.as_ref().ok_or("Scene ticks are disabled")?;
        if staged
            .config
            .borrow()
            .iter()
            .any(|request| request.name == name)
        {
            return Ok(());
        }
        let requests = self
            .configuration_requests
            .as_ref()
            .ok_or("No configuration queue")?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let request = requests
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|err| format!("configuration for {name}: {err}"))?;
            let ready = request.name == name;
            staged.config.borrow_mut().push(request);
            if ready {
                return Ok(());
            }
        }
    }

    pub fn scene_wait_join(&self, name: &str) -> Result<(), String> {
        self.scene_wait_events(
            |event| matches!(event, NetEvent::Joined(join, _) if join.name == name),
        )
        .map_err(|error| format!("play admission for {name}: {error}"))
    }

    pub fn scene_wait_pongs(
        &self,
        id: i32,
        connections: impl Iterator<Item = u32>,
    ) -> Result<(), String> {
        let mut waiting: BTreeSet<_> = connections.collect();
        if waiting.is_empty() {
            return Ok(());
        }
        self.scene_wait_events(|event| {
            if let NetEvent::Packet(conn, packet, _) = event
                && matches!(packet.as_ref(), Serverbound::Pong(pong) if pong.id == id)
            {
                waiting.remove(conn);
            }
            waiting.is_empty()
        })
        .map_err(|error| format!("play barrier {id}, waiting for {waiting:?}: {error}"))
    }

    fn scene_wait_events(&self, mut ready: impl FnMut(&NetEvent) -> bool) -> Result<(), String> {
        let staged = self.scene.as_ref().ok_or("Scene ticks are disabled")?;
        if staged
            .events
            .borrow()
            .iter()
            .any(|event| ready(event.value()))
        {
            return Ok(());
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let event = self
                .events
                .recv_timeout_owned(deadline.saturating_duration_since(Instant::now()))
                .map_err(|error| error.to_string())?;
            let ready = ready(event.value());
            staged.events.borrow_mut().push(event);
            if ready {
                return Ok(());
            }
        }
    }
}
