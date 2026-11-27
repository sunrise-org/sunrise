use sunrise_client_gameplay::game::client_side::remote_client::{
    session_state::Tracked, sync::entity_sound_state,
};
/// `ClientboundSoundEntityPacket`: the sound is heard where the entity it
/// names is — the local player included, which is no tracked entity — and
/// an id this level doesn't have plays nothing at all.
#[test]
fn a_sound_entity_is_heard_where_its_entity_is() {
    use std::collections::HashMap;
    let mut tracked = HashMap::new();
    tracked.insert(
        7,
        Tracked::new("minecraft:cow".to_owned(), [8.5, 64.0, -3.5], 0.0, 0.0),
    );
    let player = Some((2, [0.5, 70.0, 0.5], false));
    assert_eq!(
        entity_sound_state(2, player, None, &tracked, None)
            .filter(|s| !s.silent)
            .map(|s| s.position),
        Some([0.5, 70.0, 0.5]),
        "the local player is named by Remote.entity_id"
    );
    assert_eq!(
        entity_sound_state(7, player, None, &tracked, None)
            .filter(|s| !s.silent)
            .map(|s| s.position),
        Some([8.5, 64.0, -3.5]),
        "another entity the server spawned"
    );
    assert_eq!(
        entity_sound_state(9, player, None, &tracked, None)
            .filter(|s| !s.silent)
            .map(|s| s.position),
        None,
        "getEntity is null: handleSoundEntityEvent returns"
    );
    assert_eq!(
        entity_sound_state(2, None, None, &tracked, None)
            .filter(|s| !s.silent)
            .map(|s| s.position),
        None,
        "a player with no position yet"
    );
}

#[test]
fn a_sound_entity_retains_silence_for_players_and_non_mob_targets() {
    use std::collections::HashMap;
    let mut tracked = HashMap::new();
    tracked.insert(
        7,
        Tracked {
            silent: true,
            ..Tracked::new("minecraft:arrow".to_owned(), [8.5, 64.0, -3.5], 0.0, 0.0)
        },
    );
    let state = entity_sound_state(7, None, None, &tracked, None).unwrap();
    assert!(state.silent);
    assert!(!state.removed);
    assert_eq!(state.position, [8.5, 64.0, -3.5]);
    let player =
        entity_sound_state(2, Some((2, [1.0, 2.0, 3.0], true)), None, &tracked, None).unwrap();
    assert!(player.silent);
    assert!(entity_sound_state(9, None, None, &tracked, None).is_none());
}
