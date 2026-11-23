//! `Holder::Server`: one map for the whole world, reached from every level, journaled, saved
//! with the world and refused on a client.
use crate::game::native_entity_refs::tests::{with_client_game, with_game};
use crate::game::{Dim, Game};
use mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, Holder, HostTransaction, ServerHost,
};
use sunrise_core::nbt::Tag;

static COUNT: AttachmentType<i32> = AttachmentType {
    namespace: "server_test",
    name: "count",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n: &i32| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |c| match c.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

fn count(game: &mut Game<'_>) -> Option<i32> {
    COUNT.get_existing(game, Holder::Server).unwrap()
}

/// A value written while the Nether is in place is the one the Overworld reads, and a
/// level's own holder stays its own. A transaction aborted in the Nether puts it back.
#[test]
#[ignore = "daily"]
fn every_level_reaches_one_map_and_an_abort_puts_it_back() {
    with_game(|game| {
        let saves = tempfile::tempdir().unwrap();
        game.world_root = saves.path().to_path_buf();
        game.native_lifecycle.world_open_ready().unwrap();
        COUNT.set(game, Holder::Level, 1).unwrap();
        game.enter_level(Dim::Nether).unwrap();
        COUNT.set(game, Holder::Server, 7).unwrap();
        assert_eq!(COUNT.get_existing(game, Holder::Level), Ok(None));

        let host: &mut dyn ServerHost = game;
        let aborted: Result<(), _> = host.transaction(|host, _| {
            COUNT.set(host, Holder::Server, 9)?;
            assert_eq!(COUNT.get_existing(host, Holder::Server), Ok(Some(9)));
            Err(AttachmentError::MissingHolder)
        });
        assert!(aborted.is_err());
        assert_eq!(count(game), Some(7));

        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(count(game), Some(7));
        assert_eq!(COUNT.get_existing(game, Holder::Level), Ok(Some(1)));
    });
}

/// The world's save writes the map to the overworld's data folder, and a world opened again
/// reads it back.
#[test]
#[ignore = "daily"]
fn the_map_is_saved_with_the_world() {
    with_game(|game| {
        let saves = tempfile::tempdir().unwrap();
        game.world_root = saves.path().to_path_buf();
        COUNT.set(game, Holder::Server, 42).unwrap();
        game.save_server_attachments();
        assert!(
            saves
                .path()
                .join("data/sunrise/server_attachments.dat")
                .is_file()
        );
        game.server_attachments = Default::default();
        assert_eq!(count(game), Some(42));
    });
}

/// A client has no server map: reading or writing it is refused, and nothing is made for it.
#[test]
#[ignore = "daily"]
fn a_client_is_refused() {
    with_client_game(|game| {
        assert_eq!(
            COUNT.get_existing(game, Holder::Server),
            Err(AttachmentError::MissingHolder)
        );
        assert_eq!(
            COUNT.set(game, Holder::Server, 3),
            Err(AttachmentError::MissingHolder)
        );
    });
}
