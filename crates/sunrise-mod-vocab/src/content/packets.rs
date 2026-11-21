//! Owned data for client play-packet submission; protocol decoding stays in the game.

use crate::Identifier;

/// A serverbound play packet encoded in the connected game's vanilla protocol.
///
/// The game resolves the type name and validates its body once at submission. The packet
/// reserves outgoing credit and becomes visible only when the current root transaction
/// commits. A phase change or disconnect before publication cancels it. Registered and
/// engine-reserved custom payload channels require their payload/service API instead.
/// This is not a connection or protocol implementation; submission applies no local effects.
/// A `container_click`, whose carried stacks the game hashes, is built by
/// [`NetworkClientHost::container_click`](crate::NetworkClientHost::container_click).
/// Signed chat, signed commands and chat sessions have no typed builders:
/// ```compile_fail
/// mod_vocab::ServerboundPlayPacket::chat("hello");
/// ```
/// ```compile_fail
/// mod_vocab::ServerboundPlayPacket::chat_command_signed("help");
/// ```
/// ```compile_fail
/// mod_vocab::ServerboundPlayPacket::chat_session_update();
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ServerboundPlayPacket {
    /// Packet type, for example `minecraft:pong` or `minecraft:custom_payload`.
    pub id: Identifier,
    /// Vanilla packet body, without the packet id, length prefix or compression framing.
    pub body: Vec<u8>,
}

impl ServerboundPlayPacket {
    /// Construct a packet envelope; the game validates the body when it is submitted.
    pub fn new(id: Identifier, body: Vec<u8>) -> Self {
        Self { id, body }
    }
}

/// Why the client could not submit a play packet.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SendPacketError {
    /// There is no server connection.
    NotConnected,
    /// The player has not joined, or the connection is reconfiguring.
    NotInPlay,
    /// The channel belongs to a registered payload or an engine service. Use its
    /// payload/service API rather than bypassing negotiation through raw packets.
    ProtectedPayload {
        /// The protected custom-payload channel.
        id: crate::PayloadId,
    },
    /// No outgoing resource credit remains; nothing was queued.
    Backpressure,
    /// The encoded or uncompressed packet exceeds this physical connection's framing bound.
    FrameTooLarge {
        /// Packet bytes at the refused framing stage.
        size: u64,
        /// Maximum bytes accepted at that stage.
        limit: u64,
    },
    /// The physical encoder could not prepare the packet.
    EncodeFailed {
        /// The encoder's diagnostic.
        message: String,
    },
    /// This protocol has no serverbound packet with the given type name.
    UnknownPacket {
        /// The submitted type name.
        id: Identifier,
    },
    /// The body does not decode completely as the named play packet.
    InvalidBody,
    /// A custom payload exceeds vanilla's size limit.
    PayloadTooLarge {
        /// Number of payload bytes, excluding the channel name.
        size: u64,
        /// Maximum number of payload bytes vanilla accepts.
        limit: u64,
    },
}

/// Opaque identity of one connection's packet hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
// exhaustive: the private field prevents mods constructing or destructuring the id
pub struct HoldId(pub(crate) u64);
impl HoldId {
    /// Construct an identity for the game implementation.
    // hidden: game-side identity construction, matching the other opaque Wire ids
    #[doc(hidden)]
    pub fn new(value: u64) -> Self {
        Self(value)
    }
}

/// Why a hold operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum HoldError {
    /// No joined client world is available.
    NoWorld,
    /// The hold ended or belongs to a different connection.
    Unknown,
    /// Another hold is active, or this hold overflowed and was flushed.
    /// These are the configured bounds.
    Limit {
        /// Maximum held packets.
        packets: usize,
        /// Maximum held encoded bytes, including packet type ids.
        bytes: usize,
    },
    /// Publication failed. The remaining packets were discarded.
    Send {
        /// Packets successfully published before the failure.
        sent: usize,
        /// The existing packet service's refusal.
        error: SendPacketError,
    },
}
impl crate::NoBackend for HoldError {
    fn no_backend() -> Self {
        Self::NoWorld
    }
}

/// Why a client mod's `disconnect` was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DisconnectError {
    /// No server or world is joined: there is nothing to leave.
    Unavailable,
    /// The reason exceeds the text boundary's size or nesting limits.
    Text(crate::TextFormError),
}

impl From<crate::TextFormError> for DisconnectError {
    fn from(error: crate::TextFormError) -> Self {
        Self::Text(error)
    }
}

impl crate::NoBackend for DisconnectError {
    /// No game: nothing is joined.
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Reader, Writer};
    #[test]
    fn hold_errors_keep_literal_ordinals_and_partial_send_fields() {
        for (error, ordinal) in [
            (HoldError::NoWorld, 0u32),
            (HoldError::Unknown, 1),
            (
                HoldError::Limit {
                    packets: 256,
                    bytes: 1_048_576,
                },
                2,
            ),
            (
                HoldError::Send {
                    sent: 1,
                    error: SendPacketError::Backpressure,
                },
                3,
            ),
        ] {
            let mut writer = Writer::new();
            writer.put(&error);
            assert_eq!(&writer.bytes()[..4], &ordinal.to_le_bytes());
            assert_eq!(
                Reader::new(writer.bytes()).take::<HoldError>().unwrap(),
                error
            );
        }
        let mut writer = Writer::new();
        writer.put(&HoldError::Send {
            sent: 1,
            error: SendPacketError::Backpressure,
        });
        assert_eq!(
            writer.bytes(),
            &[3, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0]
        );
        let mut writer = Writer::new();
        writer.put(&HoldId::new(4178));
        assert_eq!(writer.bytes(), &[82, 16, 0, 0, 0, 0, 0, 0]);
    }
}
