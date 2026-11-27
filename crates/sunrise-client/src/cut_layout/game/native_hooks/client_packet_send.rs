#![cfg(test)]
#[cfg(test)]
mod client_packet_send_tests {
    use mod_api::{ClientHost, ClientPacketSendEvent, Flow, Hooks, ModEntry};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};
    use sunrise_protocol::packets::{
        configuration::CustomPayload,
        play::{Serverbound, SetCarriedItem},
    };

    /// Drops payloads on `example:secret`, logging every packet it hears.
    struct Filter(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Filter {
        fn client_packet_send(
            &self,
            _: &dyn ClientHost,
            event: &mut ClientPacketSendEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push(event.packet.id.to_string());
            match &event.packet.custom_payload {
                Some(payload) if payload.channel == "example:secret" => Flow::Cancel,
                _ => Flow::Continue,
            }
        }
    }

    fn payload(channel: &str) -> Serverbound {
        Serverbound::CustomPayload(CustomPayload {
            channel: channel.into(),
            data: vec![7],
        })
    }

    /// `Connection.send`: the mod hears each packet the game sends and cancels the one it
    /// filters; without a listener nothing is cancelled.
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_hears_and_cancels_packets_the_game_sends() {
        let slot = Serverbound::SetCarriedItem(SetCarriedItem { slot: 2 });
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            assert!(!game.client_packet_send_canceled(&payload("example:secret")));
        });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods =
            vec![ModEntry::new("filter", Box::new(Filter(Arc::clone(&seen))))].into();
        with_mods(mods, || {
            sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
                assert!(!game.client_packet_send_canceled(&slot));
                assert!(game.client_packet_send_canceled(&payload("example:secret")));
                assert!(!game.client_packet_send_canceled(&payload("example:open")));
            });
        });
        assert_eq!(
            *seen.lock().unwrap(),
            [
                "minecraft:set_carried_item",
                "minecraft:custom_payload",
                "minecraft:custom_payload"
            ]
        );
    }
}
