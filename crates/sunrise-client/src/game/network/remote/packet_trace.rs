//! The connection's recent packets, for the Disconnected screen's report (`Hooks::report`): what
//! this client sent and how the server corrected it, so a kick can be replayed against vanilla.
//! Kept only while a hook asks for it (`Hooks::trace_packets`, as the connection starts); otherwise
//! each packet costs one branch.
//!
//! Recording copies a few numbers into a ring allocated once: no string, no allocation, per packet.
//! The text is written only when the report is made. Chat, commands, signs, books and payloads
//! keep their name alone: what a player wrote never reaches the report.
use crate::game::Game;
use std::time::Instant;
use sunrise_protocol::packets::play::{Clientbound as C, Serverbound as S};

impl Game<'_> {
    /// A play packet going to the server (`send_play`, once a mod didn't cancel it).
    pub(in crate::game) fn trace_sent(&self, packet: &S) {
        if let Some(r) = &self.remote {
            r.packets.borrow_mut().sent(self.ticks, packet);
        }
    }
    /// A play packet the server sent, before it is handled.
    pub(super) fn trace_received(&self, packet: &C) {
        if let Some(r) = &self.remote {
            r.packets
                .borrow_mut()
                .received(self.ticks, r.entity_id, packet);
        }
    }
}

/// Packets kept: about 15 to 20 seconds of play, the client's tick end left out.
pub(super) const KEPT: usize = 400;

/// What a packet carried that tells a replay what happened.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Detail {
    Name,
    Pos([f64; 3], u8),
    PosRot([f64; 3], [f32; 2], u8),
    Rot([f32; 2], u8),
    Flags(u8),
    Vehicle([f64; 3], [f32; 2], bool),
    Action {
        action: i32,
        pos: [i32; 3],
        direction: u8,
        sequence: i32,
    },
    Command {
        action: i32,
        data: i32,
    },
    UseOn {
        hand: i32,
        pos: [i32; 3],
        face: i32,
        cursor: [f32; 3],
        sequence: i32,
    },
    Use {
        hand: i32,
        rot: [f32; 2],
        sequence: i32,
    },
    Entity(i32),
    Interact {
        id: i32,
        hand: i32,
        secondary: bool,
    },
    Slot(i16),
    Accept(i32, [f64; 3], [f32; 2]),
    Click {
        container: i32,
        state: i32,
        slot: i16,
        button: i8,
        input: i32,
    },
    /// The server moved the player: a teleport, or an anticheat's setback.
    Setback {
        teleport: i32,
        pos: [f64; 3],
        delta: [f64; 3],
        rot: [f32; 2],
        relatives: i32,
    },
    /// The server pushed the player (knockback, explosions, a plugin's velocity).
    Motion([f64; 3]),
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    ms: u32,
    tick: u64,
    sent: bool,
    name: &'static str,
    detail: Detail,
}

pub(super) struct PacketTrace {
    /// `None` while nothing asked for it.
    ring: Option<Ring>,
}

struct Ring {
    started: Instant,
    entries: Vec<Entry>,
    /// The slot the next entry overwrites once `entries` is full.
    next: usize,
}

impl PacketTrace {
    pub(super) fn new(on: bool) -> Self {
        Self {
            ring: on.then(|| Ring {
                started: Instant::now(),
                entries: Vec::with_capacity(KEPT),
                next: 0,
            }),
        }
    }

    /// A packet this client sent, at game tick `tick`.
    pub(super) fn sent(&mut self, tick: u64, packet: &S) {
        let Some(ring) = &mut self.ring else {
            return;
        };
        let detail = match packet {
            // Every tick: the tick column already shows them.
            S::ClientTickEnd(_) => return,
            S::MovePlayerPos(p) => Detail::Pos([p.x, p.y, p.z], p.flags),
            S::MovePlayerPosRot(p) => Detail::PosRot([p.x, p.y, p.z], [p.yaw, p.pitch], p.flags),
            S::MovePlayerRot(p) => Detail::Rot([p.yaw, p.pitch], p.flags),
            S::MovePlayerStatus(p) => Detail::Flags(p.flags),
            S::PlayerInput(p) => Detail::Flags(p.flags),
            S::MoveVehicle(p) => Detail::Vehicle(p.position, [p.yaw, p.pitch], p.on_ground),
            S::PlayerAction(p) => Detail::Action {
                action: p.action,
                pos: p.pos,
                direction: p.direction,
                sequence: p.sequence,
            },
            S::PlayerCommand(p) => Detail::Command {
                action: p.action,
                data: p.data,
            },
            S::UseItemOn(p) => Detail::UseOn {
                hand: p.hand,
                pos: p.pos,
                face: p.face,
                cursor: p.cursor,
                sequence: p.sequence,
            },
            S::UseItem(p) => Detail::Use {
                hand: p.hand,
                rot: [p.yaw, p.pitch],
                sequence: p.sequence,
            },
            S::Attack(p) => Detail::Entity(p.id),
            S::Interact(p) => Detail::Interact {
                id: p.id,
                hand: p.hand,
                secondary: p.secondary,
            },
            S::SetCarriedItem(p) => Detail::Slot(p.slot),
            S::AcceptTeleportation(p) => Detail::Accept(p.id, [p.x, p.y, p.z], [p.yaw, p.pitch]),
            S::ContainerClick(p) => Detail::Click {
                container: p.container,
                state: p.state,
                slot: p.slot,
                button: p.button,
                input: p.input,
            },
            _ => Detail::Name,
        };
        ring.push(tick, true, packet.name(), detail);
    }

    /// A packet the server sent that moved or pushed this client's player (`own`: its entity id).
    pub(super) fn received(&mut self, tick: u64, own: i32, packet: &C) {
        let Some(ring) = &mut self.ring else {
            return;
        };
        let detail = match packet {
            C::PlayerPosition(p) => Detail::Setback {
                teleport: p.teleport,
                pos: p.position,
                delta: p.delta,
                rot: [p.yaw, p.pitch],
                relatives: p.relatives,
            },
            C::SetEntityMotion(p) if p.id == own => Detail::Motion(p.velocity),
            _ => return,
        };
        ring.push(tick, false, packet.name(), detail);
    }

    /// The kept packets, oldest first, one line each: milliseconds since the connection started,
    /// the game tick, `>` sent or `<` received, the packet and what it carried.
    pub(super) fn lines(&self) -> Vec<String> {
        let Some(ring) = &self.ring else {
            return Vec::new();
        };
        let (newer, older) = ring.entries.split_at(ring.next.min(ring.entries.len()));
        older.iter().chain(newer).map(line).collect()
    }
}

impl Ring {
    fn push(&mut self, tick: u64, sent: bool, name: &'static str, detail: Detail) {
        let entry = Entry {
            ms: u32::try_from(self.started.elapsed().as_millis()).unwrap_or(u32::MAX),
            tick,
            sent,
            name,
            detail,
        };
        if self.entries.len() < KEPT {
            self.entries.push(entry);
        } else {
            self.entries[self.next] = entry;
        }
        self.next = (self.next + 1) % KEPT;
    }
}

/// Values print in Rust's shortest exact form, so a replay can send the same bits.
fn line(entry: &Entry) -> String {
    let arrow = if entry.sent { '>' } else { '<' };
    let head = format!("{:>8} t{} {arrow} {}", entry.ms, entry.tick, entry.name);
    let detail = match entry.detail {
        Detail::Name => String::new(),
        Detail::Pos([x, y, z], flags) => format!("{x} {y} {z} f{flags}"),
        Detail::PosRot([x, y, z], [yaw, pitch], flags) => {
            format!("{x} {y} {z} {yaw} {pitch} f{flags}")
        }
        Detail::Rot([yaw, pitch], flags) => format!("{yaw} {pitch} f{flags}"),
        Detail::Flags(flags) => format!("f{flags:#010b}"),
        Detail::Vehicle([x, y, z], [yaw, pitch], on_ground) => {
            format!("{x} {y} {z} {yaw} {pitch} ground={on_ground}")
        }
        Detail::Action {
            action,
            pos: [x, y, z],
            direction,
            sequence,
        } => format!("action={action} {x} {y} {z} dir={direction} seq={sequence}"),
        Detail::Command { action, data } => format!("action={action} data={data}"),
        Detail::UseOn {
            hand,
            pos: [x, y, z],
            face,
            cursor: [cx, cy, cz],
            sequence,
        } => format!("hand={hand} {x} {y} {z} face={face} cursor={cx} {cy} {cz} seq={sequence}"),
        Detail::Use {
            hand,
            rot: [yaw, pitch],
            sequence,
        } => format!("hand={hand} {yaw} {pitch} seq={sequence}"),
        Detail::Entity(id) => format!("entity={id}"),
        Detail::Interact {
            id,
            hand,
            secondary,
        } => format!("entity={id} hand={hand} secondary={secondary}"),
        Detail::Slot(slot) => format!("slot={slot}"),
        Detail::Accept(id, [x, y, z], [yaw, pitch]) => {
            format!("id={id} {x} {y} {z} {yaw} {pitch}")
        }
        Detail::Click {
            container,
            state,
            slot,
            button,
            input,
        } => {
            format!("container={container} state={state} slot={slot} button={button} input={input}")
        }
        Detail::Setback {
            teleport,
            pos: [x, y, z],
            delta: [dx, dy, dz],
            rot: [yaw, pitch],
            relatives,
        } => format!(
            "id={teleport} {x} {y} {z} delta={dx} {dy} {dz} {yaw} {pitch} relatives={relatives:#x}"
        ),
        Detail::Motion([x, y, z]) => format!("{x} {y} {z}"),
    };
    if detail.is_empty() {
        head
    } else {
        format!("{head} {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_protocol::packets::play;

    fn pos(x: f64) -> S {
        S::MovePlayerPos(play::MovePlayerPos {
            x,
            y: 64.0,
            z: 0.5,
            flags: 1,
        })
    }

    /// The ring keeps the newest packets in order, a setback and the player's own knockback
    /// come in, chat keeps its name alone, and nothing is kept while it is off.
    #[test]
    fn the_trace_keeps_the_newest_packets_without_what_was_written() {
        let mut trace = PacketTrace::new(true);
        for i in 0..KEPT + 3 {
            trace.sent(i as u64, &pos(i as f64));
            trace.sent(
                i as u64,
                &S::ClientTickEnd(sunrise_protocol::packets::Empty),
            );
        }
        trace.sent(
            1000,
            &S::ChatCommand(play::ChatCommand {
                command: "msg Alex my base is at 100 200".into(),
            }),
        );
        trace.received(
            1001,
            7,
            &C::PlayerPosition(play::PlayerPosition {
                teleport: 3,
                position: [1.5, 70.0, -2.25],
                delta: [0.0; 3],
                yaw: 90.0,
                pitch: 0.0,
                relatives: 0,
            }),
        );
        let knockback = |id| {
            C::SetEntityMotion(play::SetEntityMotion {
                id,
                velocity: [0.25, 0.4, 0.0],
            })
        };
        trace.received(1002, 7, &knockback(7));
        trace.received(1002, 7, &knockback(8));
        let lines = trace.lines();
        assert_eq!(lines.len(), KEPT);
        let tail = |line: &str| line.split_once(" t").unwrap().1.to_owned();
        assert_eq!(tail(&lines[0]), "6 > MovePlayerPos 6 64 0.5 f1");
        assert_eq!(
            tail(&lines[KEPT - 4]),
            format!("{} > MovePlayerPos {} 64 0.5 f1", KEPT + 2, KEPT + 2)
        );
        assert_eq!(tail(&lines[KEPT - 3]), "1000 > ChatCommand");
        assert_eq!(
            tail(&lines[KEPT - 2]),
            "1001 < PlayerPosition id=3 1.5 70 -2.25 delta=0 0 0 90 0 relatives=0x0"
        );
        assert_eq!(tail(&lines[KEPT - 1]), "1002 < SetEntityMotion 0.25 0.4 0");
        let mut off = PacketTrace::new(false);
        off.sent(0, &pos(0.0));
        assert!(off.lines().is_empty());
    }
}
