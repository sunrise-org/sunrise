//! Actor-owned holds. Overflow and partial publication are isolated in Holds.
use super::Game;
use mod_api::{HoldError, HoldId, SendPacketError, ServerboundPlayPacket};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use sunrise_protocol::{buf::Writer, packets::play::Serverbound as S};

const PACKETS: usize = 256;
const BYTES: usize = 1_048_576;
static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub(in crate::game) struct Holds {
    incarnation: Option<(u64, u64)>,
    active: Option<(HoldId, Vec<Vec<u8>>, usize)>,
    overflowed: HashSet<HoldId>,
}
impl Holds {
    pub(in crate::game) fn clear(&mut self) {
        *self = Self::default();
    }
    fn sync(&mut self, incarnation: Option<(u64, u64)>) {
        if self.incarnation != incarnation {
            self.clear();
            self.incarnation = incarnation;
        }
    }
    fn start(&mut self) -> Result<HoldId, HoldError> {
        if self.incarnation.is_none() {
            return Err(HoldError::NoWorld);
        }
        if self.active.is_some() {
            return Err(Self::limit());
        }
        let id = HoldId::new(NEXT.fetch_add(1, Ordering::Relaxed));
        self.active = Some((id, Vec::new(), 0));
        Ok(id)
    }
    fn take(&mut self, id: HoldId) -> Result<Vec<Vec<u8>>, HoldError> {
        if self.overflowed.remove(&id) {
            return Err(Self::limit());
        }
        if self.active.as_ref().is_none_or(|a| a.0 != id) {
            return Err(HoldError::Unknown);
        }
        Ok(self.active.take().unwrap().1)
    }
    fn limit() -> HoldError {
        HoldError::Limit {
            packets: PACKETS,
            bytes: BYTES,
        }
    }
    // Overflow policy: end and flush, then report Limit on the id once.
    fn queue(
        &mut self,
        body: Vec<u8>,
        send: impl FnMut(Vec<u8>) -> Result<(), SendPacketError>,
    ) -> Result<usize, HoldError> {
        let Some((id, packets, bytes)) = self.active.as_mut() else {
            return Self::publish(vec![body], send);
        };
        if packets.len() < PACKETS && body.len() <= BYTES.saturating_sub(*bytes) {
            *bytes += body.len();
            packets.push(body);
            return Ok(0);
        }
        self.overflowed.insert(*id);
        let mut packets = self.active.take().unwrap().1;
        packets.push(body);
        let result = Self::publish(packets, send);
        if matches!(
            &result,
            Err(HoldError::Send {
                error: SendPacketError::NotInPlay | SendPacketError::NotConnected,
                ..
            })
        ) {
            self.clear();
        }
        result
    }
    // Send refusal policy: discard the tail and report the published prefix.
    fn publish(
        packets: Vec<Vec<u8>>,
        mut send: impl FnMut(Vec<u8>) -> Result<(), SendPacketError>,
    ) -> Result<usize, HoldError> {
        let count = packets.len();
        for (sent, body) in packets.into_iter().enumerate() {
            send(body).map_err(|error| HoldError::Send { sent, error })?;
        }
        Ok(count)
    }
}

impl Game<'_> {
    fn hold_incarnation(&self) -> Option<(u64, u64)> {
        self.remote
            .as_ref()
            .filter(|r| r.joined && r.reconfiguring.is_none())?
            .session
            .hold_incarnation()
    }
    pub(in crate::game) fn hold_packets(&mut self) -> Result<HoldId, HoldError> {
        let mut holds = self.native_net.holds.borrow_mut();
        holds.sync(self.hold_incarnation());
        holds.start()
    }
    pub(in crate::game) fn release_held(&mut self, hold: HoldId) -> Result<usize, HoldError> {
        let (incarnation, packets) = {
            let mut holds = self.native_net.holds.borrow_mut();
            holds.sync(self.hold_incarnation());
            let packets = holds.take(hold)?;
            (holds.incarnation.ok_or(HoldError::Unknown)?, packets)
        };
        let count = packets.len();
        let remote = self.remote.as_ref().ok_or(HoldError::Unknown)?;
        Holds::publish(packets, |body| {
            remote.session.send_held_body(body, incarnation)
        })?;
        Ok(count)
    }
    pub(in crate::game) fn drop_held(&mut self, hold: HoldId) -> Result<(), HoldError> {
        let mut holds = self.native_net.holds.borrow_mut();
        holds.sync(self.hold_incarnation());
        holds.take(hold).map(|_| ())
    }
    pub(in crate::game) fn send_or_hold_play(&self, packet: S) {
        let Some(remote) = &self.remote else {
            return;
        };
        let mut holds = self.native_net.holds.borrow_mut();
        holds.sync(self.hold_incarnation());
        if holds.active.is_none() {
            remote.session.send(packet);
            return;
        }
        let incarnation = holds.incarnation.expect("active hold has an incarnation");
        if let Ok(body) = packet.try_encode() {
            let _ = holds.queue(body, |body| {
                remote.session.send_held_body(body, incarnation)
            });
        }
    }
    pub(in crate::game) fn publish_mod_held(
        &self,
        prepared: sunrise_client::net::PreparedPacket,
        packet: ServerboundPlayPacket,
    ) {
        if !prepared.is_current() {
            return;
        }
        let mut holds = self.native_net.holds.borrow_mut();
        holds.sync(self.hold_incarnation());
        if holds.active.is_none() {
            prepared.publish();
            return;
        }
        // The envelope has already passed the packet service's validation and reservation.
        // Drop its reservation before applying the hold's own bounded storage.
        drop(prepared);
        let Some(remote) = &self.remote else {
            return;
        };
        let id = sunrise_protocol::ids::play::serverbound::NAMES
            .iter()
            .position(|id| *id == packet.id.path())
            .unwrap();
        let mut body = Writer::new();
        body.varint(id as i32);
        body.bytes(&packet.body);
        let incarnation = holds.incarnation.expect("active hold has an incarnation");
        let _ = holds.queue(body.data, |body| {
            remote.session.send_held_body(body, incarnation)
        });
    }
}
impl Game<'_> {
    /// `Connection.send` for the game's play packets: a mod may cancel one
    /// (`client_packet_send`), and the server never hears of it. True when queued.
    pub(in crate::game) fn send_play(&self, packet: S) -> bool {
        let queued = self.remote.is_some() && !self.client_packet_send_canceled(&packet);
        if queued {
            self.trace_sent(&packet);
            self.send_or_hold_play(packet);
        }
        queued
    }
}

#[cfg(test)]
#[path = "packet_hold_tests.rs"]
mod tests;
