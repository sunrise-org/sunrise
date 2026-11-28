//! `set_local_rotation`: a client mod turns the local player's view; the look goes out in the
//! tick's own player sync, one move packet with vanilla's clamp, never a packet of the call's own.
use sunrise_protocol::connection::Connection;
use sunrise_protocol::packets::play::Serverbound as S;

/// The move packets the client sent before a fence, as (kind, yaw, pitch).
#[cfg(feature = "client")]
pub(crate) fn sent_moves(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    server: &mut Connection,
) -> Vec<(&'static str, f32, f32)> {
    game.send(S::Punch(sunrise_protocol::packets::Empty));
    let mut moves = Vec::new();
    loop {
        let (id, payload) = server.recv().unwrap();
        match S::decode(id, &payload).unwrap() {
            S::MovePlayerPosRot(p) => moves.push(("pos_rot", p.yaw, p.pitch)),
            S::MovePlayerRot(p) => moves.push(("rot", p.yaw, p.pitch)),
            S::MovePlayerPos(_) => moves.push(("pos", f32::NAN, f32::NAN)),
            S::MovePlayerStatus(_) => moves.push(("status", f32::NAN, f32::NAN)),
            S::Punch(_) => return moves,
            _ => {}
        }
    }
}

/// How many packets of any kind the client sent before a fence.
#[cfg(feature = "client")]
pub(crate) fn sent_packets(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    server: &mut Connection,
) -> usize {
    game.send(S::Punch(sunrise_protocol::packets::Empty));
    let mut count = 0;
    loop {
        let (id, payload) = server.recv().unwrap();
        if matches!(S::decode(id, &payload).unwrap(), S::Punch(_)) {
            return count;
        }
        count += 1;
    }
}

/// The `PlayerInput` and `PlayerCommand` packets the client sent before a fence, in order, as
/// `player_input:<payload hex>` and `player_command:<payload hex>`.
#[cfg(feature = "client")]
pub(crate) fn sent_input(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    server: &mut Connection,
) -> Vec<String> {
    game.send(S::Punch(sunrise_protocol::packets::Empty));
    let mut sent = Vec::new();
    loop {
        let (id, payload) = server.recv().unwrap();
        let kind = match S::decode(id, &payload).unwrap() {
            S::PlayerInput(_) => "player_input",
            S::PlayerCommand(_) => "player_command",
            S::Punch(_) => return sent,
            _ => continue,
        };
        let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
        sent.push(format!("{kind}:{hex}"));
    }
}

/// `sent_input.json` from `tools/test_network_parity.py`'s vanilla oracle.
#[cfg(feature = "client")]
pub(crate) fn sent_input_witness() -> serde_json::Value {
    let dir = std::env::var("SUNRISE_NETWORK_WITNESS_DIR")
        .expect("SUNRISE_NETWORK_WITNESS_DIR must be set (run via tools/test_network_parity.py)");
    let path = std::path::Path::new(&dir).join("sent_input.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}
