use super::super::{ChatTrust, QueuedChat};
use crate::game::{Game, native_entity_refs::tests::with_client_game};
use mod_api::prelude::NetworkClientHost;
use mod_api::{ClientHost, ClientLocalChatLinePreEvent, Flow, Hooks, LocalChatSource, ModEntry};
use std::sync::{Arc, Mutex};
use sunrise_client::chat::{ChatAbilities, ChatVisibility};
use sunrise_client::text::Text;
use sunrise_core::nbt::Tag;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound},
    secure_chat::Signature,
};

struct Decorate(LocalChatSource);

#[mod_api::__subscriptions]
impl Hooks for Decorate {
    fn client_local_chat_line_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut ClientLocalChatLinePreEvent<'_>,
    ) -> Flow {
        if event.source() == self.0 {
            event
                .message_mut()
                .children
                .push(mod_api::Text::literal("!"));
            event.message_mut().style.bold = Some(true);
        }
        Flow::Continue
    }
}

struct EmitNotice;

#[mod_api::__subscriptions]
impl Hooks for EmitNotice {
    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut mod_api::ClientTickPreEvent) {
        host.chat_line(mod_api::Text::literal("mod notice"))
            .unwrap();
    }
}

fn with_chat_mods(mods: Vec<ModEntry>, test: impl FnOnce(&mut Game<'_>)) {
    mod_platform::testing::with_mods(mods.into(), || with_client_game(test));
}

fn connect(game: &mut Game<'_>) -> Pipe {
    let (client, server) = Pipe::pair();
    game.connect_local(client, "ChatLineTest").unwrap();
    server
}

#[test]
#[ignore = "daily"]
fn decorates_a_game_notice_without_a_connection() {
    with_chat_mods(
        vec![ModEntry::new(
            "decorate",
            Box::new(Decorate(LocalChatSource::Server)),
        )],
        |game| {
            assert!(game.remote.is_none());
            game.system_message(Text::literal("game notice"));
            assert_eq!(game.ui.chat.recent_lines(2), ["game notice!"]);
            assert_eq!(game.ui.chat.recent_components(1)[0]["bold"], true);
        },
    );
}

#[test]
#[ignore = "daily"]
fn intercepts_another_mods_notice_once_while_connected() {
    with_chat_mods(
        vec![
            ModEntry::new("emitter", Box::new(EmitNotice)),
            ModEntry::new("decorate", Box::new(Decorate(LocalChatSource::Client))),
        ],
        |game| {
            let _server = connect(game);
            game.ui.chat.set_abilities(
                &game.gui.atlas.font,
                ChatAbilities::new(ChatVisibility::Hidden),
            );
            game.native_client_tick_pre();
            game.ui.chat.rescale(&game.gui.atlas);
            assert_eq!(game.ui.chat.recent_lines(2), ["mod notice!"]);
            let remote = game.remote.as_ref().unwrap();
            assert_eq!(remote.chat_received, 1);
            assert_eq!(remote.chat[0].0, "mod notice!");
        },
    );
}

#[test]
#[ignore = "daily"]
fn hud_only_notices_keep_their_local_accounting() {
    with_chat_mods(
        vec![ModEntry::new(
            "decorate",
            Box::new(Decorate(LocalChatSource::Client)),
        )],
        |game| {
            let _server = connect(game);
            game.ui.screenshot_messages.push("capture.png".into());
            game.finish_screenshot(&Ok([1, 1]), None);
            game.native_chat("native notice");
            let components = game.ui.chat.recent_components(2);
            assert_eq!(game.ui.chat.recent_lines(1), ["native notice!"]);
            assert_eq!(components[1]["translate"], "screenshot.success");
            assert_eq!(
                components[1]["with"][0],
                serde_json::json!({
                    "text": "capture.png",
                    "underlined": true,
                    "click_event": {"action": "open_file", "path": "capture.png"},
                })
            );
            assert!(components[1].get("bold").is_none());
            assert_eq!(game.ui.chat.received, 2);
            assert_eq!(game.remote.as_ref().unwrap().chat_received, 0);
        },
    );
}

struct First;

#[mod_api::__subscriptions]
impl Hooks for First {
    #[listener(priority = 10)]
    fn client_local_chat_line_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut ClientLocalChatLinePreEvent<'_>,
    ) -> Flow {
        event
            .message_mut()
            .children
            .push(mod_api::Text::literal("A"));
        Flow::Continue
    }
}

struct Mark(&'static str);

#[mod_api::__subscriptions]
impl Hooks for Mark {
    fn client_local_chat_line_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut ClientLocalChatLinePreEvent<'_>,
    ) -> Flow {
        event
            .message_mut()
            .children
            .push(mod_api::Text::literal(self.0));
        if self.0 == "B" && event.source() == LocalChatSource::Server {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

struct ObserveCancelled(Arc<Mutex<Vec<(bool, usize)>>>);

#[mod_api::__subscriptions]
impl Hooks for ObserveCancelled {
    #[listener(priority = -10, receive_cancelled)]
    fn client_local_chat_line_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut ClientLocalChatLinePreEvent<'_>,
    ) -> Flow {
        self.0
            .lock()
            .unwrap()
            .push((event.cancelled, event.message().children.len()));
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn listener_order_and_cancellation_control_history_and_accounting() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    with_chat_mods(
        vec![
            // B and C have equal priority and retain the supplied load order.
            ModEntry::new("mark-b", Box::new(Mark("B"))),
            ModEntry::new("mark-c", Box::new(Mark("C"))),
            ModEntry::new("first", Box::new(First)),
            ModEntry::new("observer", Box::new(ObserveCancelled(seen.clone()))),
        ],
        |game| {
            let _server = connect(game);
            game.chat_line(Text::literal("keep"));
            game.system_message(Text::literal("cancel"));
            game.ui.chat.set_abilities(
                &game.gui.atlas.font,
                ChatAbilities::new(ChatVisibility::Hidden),
            );
            game.system_message(Text::literal("filtered before dispatch"));
            assert_eq!(*seen.lock().unwrap(), [(false, 3), (true, 2)]);
            assert_eq!(game.ui.chat.recent_lines(3), ["keepABC"]);
            assert_eq!(game.ui.chat.received, 1);
            let remote = game.remote.as_ref().unwrap();
            assert_eq!(remote.chat_received, 1);
            assert_eq!(remote.chat[0].0, "keepABC");
        },
    );
}

fn invalid_text() -> mod_api::Text {
    let mut text = mod_api::Text::literal("too deep");
    for _ in 0..32_768 {
        text = mod_api::Text::literal("").append(text);
    }
    text
}

struct InvalidReplacement;

#[mod_api::__subscriptions]
impl Hooks for InvalidReplacement {
    fn client_local_chat_line_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut ClientLocalChatLinePreEvent<'_>,
    ) -> Flow {
        *event.message_mut() = invalid_text();
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn invalid_replacement_keeps_original() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    with_chat_mods(
        vec![
            ModEntry::new("invalid", Box::new(InvalidReplacement)),
            ModEntry::new("observer", Box::new(ObserveCancelled(seen.clone()))),
        ],
        |game| {
            game.system_message(Text::literal("original"));
            assert_eq!(*seen.lock().unwrap(), [(false, 1)]);
            assert_eq!(game.ui.chat.recent_lines(2), ["original"]);
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_game_line_that_cannot_cross_the_boundary_is_still_shown() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    with_chat_mods(
        vec![ModEntry::new(
            "observer",
            Box::new(ObserveCancelled(seen.clone())),
        )],
        |game| {
            // An unknown component field is valid engine JSON but cannot be
            // represented as an NBT compound with this nested null.
            game.system_message(Text::from(serde_json::json!({
                "text": "engine line",
                "unknown": { "value": null },
            })));
            assert_eq!(game.ui.chat.recent_lines(2), ["engine line"]);
            assert!(seen.lock().unwrap().is_empty());
        },
    );
}

#[test]
#[ignore = "daily"]
fn invalid_host_input_is_refused_before_insertion() {
    with_client_game(|game| {
        assert_eq!(
            NetworkClientHost::chat_line(game, invalid_text()),
            Err(mod_api::TextFormError::TooDeep)
        );
        assert!(game.ui.chat.recent_lines(1).is_empty());
        assert_eq!(game.ui.chat.received, 0);
    });
}

struct Unwind;

#[mod_api::__subscriptions]
impl Hooks for Unwind {
    fn client_local_chat_line_pre(
        &self,
        _: &dyn ClientHost,
        event: &mut ClientLocalChatLinePreEvent<'_>,
    ) -> Flow {
        *event.message_mut() = mod_api::Text::literal("discard me");
        event.cancelled = true;
        panic!("local chat listener failed");
    }
}

#[test]
#[ignore = "daily"]
fn a_failed_listener_discards_only_its_own_edits() {
    with_chat_mods(
        vec![
            ModEntry::new("first", Box::new(First)),
            ModEntry::new("unwind", Box::new(Unwind)),
            ModEntry::new("mark-c", Box::new(Mark("C"))),
        ],
        |game| {
            game.system_message(Text::literal("original"));
            assert_eq!(game.ui.chat.recent_lines(2), ["originalAC"]);
        },
    );
}

#[test]
#[ignore = "daily"]
fn transaction_rollback_discards_a_staged_local_notice() {
    with_chat_mods(
        vec![ModEntry::new(
            "decorate",
            Box::new(Decorate(LocalChatSource::Client)),
        )],
        |game| {
            let _server = connect(game);
            let root = game.transaction_open();
            let nested = game.transaction_open();
            NetworkClientHost::chat_line(game, mod_api::Text::literal("discarded")).unwrap();
            game.transaction_close(&nested, true);
            assert!(game.ui.chat.recent_lines(1).is_empty());
            assert_eq!(game.remote.as_ref().unwrap().chat_received, 0);
            game.transaction_close(&root, false);
            assert!(game.ui.chat.recent_lines(1).is_empty());
            assert_eq!(game.remote.as_ref().unwrap().chat_received, 0);

            let root = game.transaction_open();
            NetworkClientHost::chat_line(game, mod_api::Text::literal("committed")).unwrap();
            assert!(game.ui.chat.recent_lines(1).is_empty());
            game.transaction_close(&root, true);
            assert_eq!(game.ui.chat.recent_lines(2), ["committed!"]);
            assert_eq!(game.remote.as_ref().unwrap().chat_received, 1);
        },
    );
}

#[test]
#[ignore = "daily"]
fn received_chat_and_signature_deletion_bypass_the_local_hook() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    with_chat_mods(
        vec![ModEntry::new(
            "observer",
            Box::new(ObserveCancelled(seen.clone())),
        )],
        |game| {
            let _server = connect(game);
            game.handle_packet(Clientbound::SystemChat(play::SystemChat {
                content: Tag::String("server notice".into()),
                overlay: false,
            }))
            .unwrap();
            // Start at ChatListener with a signature already accepted by the connection.
            let signature: Signature = [19; 256];
            game.handle_player_chat(QueuedChat {
                line: Text::literal("signed line"),
                signature: Some(signature),
                trust: ChatTrust::Secure,
                narration: "signed line".into(),
                sender: 23,
                chat_type: 0,
                player: None,
            });
            assert_eq!(
                game.ui.chat.recent_lines(2),
                ["signed line", "server notice"]
            );
            let delete_at = game.visual_ticks() + 60;
            game.handle_packet(Clientbound::DeleteChat(play::DeleteChat {
                signature: (-1, Some(signature.to_vec())),
            }))
            .unwrap();
            game.ui.chat.tick(&game.gui.atlas, delete_at - 1);
            assert_eq!(
                game.ui.chat.recent_lines(2),
                ["signed line", "server notice"]
            );
            game.ui.chat.tick(&game.gui.atlas, delete_at);
            assert_eq!(
                game.ui.chat.recent_components(1)[0]["translate"],
                "chat.deleted_marker"
            );
            assert_eq!(game.ui.chat.recent_lines(2)[1], "server notice");
            assert!(seen.lock().unwrap().is_empty());
        },
    );
}
