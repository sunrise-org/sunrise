//! The game-side contract for starting and supervising a dedicated server.

/// The first dedicated launch contract advertised by `--launch-api`.
pub const CONTRACT: u32 = 2;

/// The local control server's endpoint file (TCP port and token), `auth` handshake,
/// and text requests with one JSON reply per line. See `sunrise-client`'s `control.rs`.
pub const CONTROL_PROTOCOL: u32 = 1;

/// The dedicated server's stdout log, including the INFO prefix and the `Done (`,
/// `Stopping server` and `Stopping in ` messages the supervisor acts on.
pub const LOG_FORMAT: u32 = 1;

pub const RESOURCES: &str = "--resources";
pub const CONTROL_SOCKET: &str = "--control-socket";
pub const TRANSFERS: &str = "--transfers";
pub const DEDICATED: &str = "--dedicated";
pub const NOGUI: &str = "--nogui";

/// The dedicated options exposed by the game and used by the supervisor.
/// `DEDICATED` separates Sunrise's options from vanilla's server options.
pub const OPTIONS: &[&str] = &[RESOURCES, CONTROL_SOCKET, TRANSFERS, DEDICATED, NOGUI];
