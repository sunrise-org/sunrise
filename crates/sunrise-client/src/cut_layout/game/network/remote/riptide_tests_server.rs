use sunrise_client_gameplay::game::remote::riptide_tests::OWNER;
use sunrise_protocol::connection::ConnReader;
use sunrise_protocol::packets::Empty;
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_sim::lan::guest::InboxItem;

/// Transfer the client's real play packets to the integrated server's normal
/// connection drain. The test drives ticks without scheduling two game loops.
pub(crate) fn client_tick_to_server(
    client: &sunrise_client_gameplay::game::ClientGame<'_>,
    peer: &mut ConnReader,
    server: &mut sunrise_gameplay::game::ServerGame<'_>,
) -> Vec<S> {
    client.send(S::ClientTickEnd(Empty));
    let mut packets = Vec::new();
    loop {
        let (id, bytes) = peer.recv().unwrap();
        let packet = S::decode(id, &bytes).unwrap();
        let end = matches!(packet, S::ClientTickEnd(_));
        packets.push(packet);
        if end {
            break;
        }
    }
    let hub = &mut server.server.lan.as_mut().unwrap().hub;
    hub.update_guest(OWNER, |g| {
        g.inbox
            .extend(packets.iter().cloned().map(|p| InboxItem::Packet(p, None)));
    })
    .unwrap();
    server.server.level_pass = Default::default();
    server.tick_lan_packets();
    let hub = &mut server.server.lan.as_mut().unwrap().hub;
    hub.end_tick();
    hub.guest(OWNER).unwrap().sender.flush();
    packets
}
