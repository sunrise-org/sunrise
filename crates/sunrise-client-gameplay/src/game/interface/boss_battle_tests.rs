//! `bossBattle` (spec 5.2, decision D-12) follows the boss bars a server sends: the packets go
//! through `handle_packet`, and the value is read from `shader_pack_inputs`, the frame's inputs
//! to the pack's uniforms. The case runs in a process of its own (one game, its own resources).
use super::native_entity_refs::tests::with_client_game;
use sunrise_core::nbt::Tag;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{BossAction, BossEvent, Clientbound},
};

const VERTEX: &str = "#version 120\nvarying vec2 texcoord;\nvoid main() {\n    gl_Position = ftransform();\n    texcoord = gl_MultiTexCoord0.xy;\n}\n";
const FRAGMENT: &str = "#version 120\nuniform sampler2D colortex0;\nuniform int bossBattle;\nvarying vec2 texcoord;\nvoid main() {\n    gl_FragColor = texture2D(colortex0, texcoord) + float(bossBattle);\n}\n";

/// The event vanilla's `WitherBoss` (purple, progress, darkening), `EnderDragonFight` (pink,
/// progress, music and fog) and `/bossbar add` (white, progress) make.
fn add(name: &str, color: i32, darken: bool, music: bool, fog: bool) -> BossAction {
    BossAction::Add {
        name: Tag::String(name.into()),
        progress: 1.0,
        color,
        overlay: 0,
        darken,
        music,
        fog,
    }
}

#[test]
#[ignore = "daily"]
fn boss_battle_follows_the_servers_boss_bars() {
    sunrise_gameplay_common::game::in_own_process(
        "game::client_side::boss_battle_tests::boss_battle_follows_the_servers_boss_bars",
        || {
            // The frame's pack inputs are filled only while a pack is picked.
            let scratch = tempfile::tempdir().unwrap();
            let pack = scratch.path().join("boss-battle");
            let shaders = pack.join("shaders");
            std::fs::create_dir_all(&shaders).unwrap();
            std::fs::write(shaders.join("final.vsh"), VERTEX).unwrap();
            std::fs::write(shaders.join("final.fsh"), FRAGMENT).unwrap();
            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "BossBattleTest").unwrap();
                game.client.settings.sunrise.shader_pack = pack.to_str().unwrap().to_owned();
                let sky = sunrise_level::environment::Sky::default();
                let camera = glam::DVec3::new(0.5, 65.5, 0.5);
                // The first frame turns the resource packs' material maps on and reloads them.
                game.shader_pack_inputs(camera, camera, 0.0, &sky);
                let read = |game: &mut crate::game::ClientGame<'_>,
                            packet: Option<(u128, BossAction)>| {
                    if let Some((id, action)) = packet {
                        game.handle_packet(Clientbound::BossEvent(BossEvent { id, action }))
                            .unwrap();
                    }
                    game.shader_pack_inputs(camera, camera, 0.0, &sky)
                        .boss_battle
                };
                assert_eq!(read(game, None), 0, "no bar");
                assert_eq!(
                    read(game, Some((1, add("Demo", 6, false, false, false)))),
                    1
                );
                assert_eq!(
                    read(game, Some((2, add("Wither", 5, true, false, false)))),
                    3
                );
                assert_eq!(read(game, Some((3, add("Bob", 0, false, true, true)))), 3);
                assert_eq!(
                    read(game, Some((2, BossAction::Remove))),
                    2,
                    "the dragon after the wither"
                );
                assert_eq!(
                    read(game, Some((3, BossAction::Remove))),
                    1,
                    "the custom bar is left"
                );
                assert_eq!(read(game, Some((1, BossAction::Remove))), 0);
            });
        },
    );
}
