//! Viewed levels, client side (#3716): the answer to a Sunrise server's `sunrise:levels` query in
//! configuration (`sunrise_protocol::viewed_levels`), sent before the client acknowledges
//! `FinishConfiguration`. The server sends a client nothing more on the channel than its answer
//! allows; the game keeps a world for each level it opens (`game::viewed_worlds`).
use sunrise_protocol::{packets::configuration as cfg, viewed_levels};

/// The bounded operational capacity advertised by this client, besides its own level.
/// Four slots support independent runtime-level projections without allocating worlds until
/// an admitted `open`. The same capacity is enforced by the play router. Server area, column,
/// entity and projection quotas still apply; older clients retain their own advertised cap.
pub const MAX_LEVELS: u8 = 4;

/// Whether `payload` is the server's viewed-levels query.
pub fn asked(payload: &cfg::CustomPayload) -> bool {
    payload.channel == viewed_levels::CHANNEL
}

/// The answer to the query `data`, or the reason that closes the connection.
pub fn answer(data: &[u8]) -> Result<cfg::Serverbound, String> {
    viewed_levels::read_query(data).map_err(|error| error.to_string())?;
    Ok(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
        channel: viewed_levels::CHANNEL.into(),
        data: viewed_levels::answer(MAX_LEVELS),
    }))
}
