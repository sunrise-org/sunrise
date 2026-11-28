use super::*;

#[test]
fn the_header_and_deep_rows_share_one_scrolled_viewport() {
    let view = View::new((800.0, 240.0), 53).unwrap();
    let mut scroll = ui::Scroll::default();
    scroll.amount = 10_000.0;
    let offset = view.offset(scroll);
    assert_eq!(offset, view.content - (view.bottom - TOP));
    assert!(
        view.clip([300.0, TOP + PADDING - offset, 100.0, HEADER])
            .is_none()
    );
    assert!(view.row_y(52, offset) < view.bottom);
    assert!(view.clip([300.0, TOP - 5.0, 100.0, 20.0]).is_some());
    assert!(view.clip([300.0, view.bottom + 1.0, 100.0, 20.0]).is_none());
}

#[test]
fn uuid_actions_are_typed_and_operator_state_changes_the_action() {
    let player = PlayerInfo {
        uuid: Some("0000000000000000000000000000002a".into()),
        ..Default::default()
    };
    let id = player_uuid(&player).unwrap();
    assert_eq!(
        PlayerColumn::Operator.action(id, false),
        PlayerAction::Op(id)
    );
    assert_eq!(
        PlayerColumn::Operator.action(id, true),
        PlayerAction::Deop(id)
    );
    assert_eq!(
        PlayerColumn::Remove.action(id, false),
        PlayerAction::Remove(id)
    );
}

#[test]
fn sorting_places_accepted_players_first_then_names_without_case() {
    let mut players = vec![
        PlayerInfo {
            name: "zulu".into(),
            accepted: true,
            ..Default::default()
        },
        PlayerInfo {
            name: "Ada".into(),
            accepted: false,
            ..Default::default()
        },
        PlayerInfo {
            name: "bea".into(),
            accepted: true,
            ..Default::default()
        },
    ];
    super::super::client::sort_invited(&mut players);
    assert_eq!(
        players.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["bea", "zulu", "Ada"]
    );
}

#[test]
fn click_hit_is_invalid_after_scroll_or_size_changes() {
    let view = View::new((800.0, 240.0), 20).unwrap();
    let frame = Frame {
        realm: 4,
        view,
        offset: 0.0,
        heading: None,
        controls: vec![Control {
            rect: [500.0, 44.0, 30.0, 20.0],
            row: 0,
            action: PlayerAction::Op(9),
        }],
    };
    assert!(hits((510.0, 50.0), frame.controls[0].rect));
    let mut scroll = ui::Scroll::default();
    scroll.amount = 1.0;
    assert_ne!(view.offset(scroll), frame.offset);
    assert_ne!(View::new((700.0, 240.0), 20).unwrap(), frame.view);
}

const SIZE: (f32, f32) = (800.0, 240.0);

fn draw(screen: &Screen) {
    let source = sunrise_assets::resource_source::ResourceSource::indexed(Default::default());
    let atlas = HudAtlas::build(&source).unwrap();
    let mut batch = HudBatch::scratch([800, 240]);
    screen.draw_players(&mut batch, &atlas, (0.0, 0.0), SIZE, true);
}

#[test]
fn focus_reaches_the_53rd_player_and_survives_reordering_by_uuid() {
    let mut screen = super::super::tests::loaded_players(53);
    draw(&screen);
    for _ in 0..53 {
        let rect = screen
            .move_player_focus(KeyCode::ArrowDown, SIZE)
            .expect("visible destination");
        assert_eq!(rect[3], 20.0);
    }
    let [x, y, w, h] = screen.move_player_focus(KeyCode::ArrowRight, SIZE).unwrap();
    let point = (x + w / 2.0, y + h / 2.0);
    assert_eq!(screen.player_control_at(point, SIZE), None);
    draw(&screen);
    assert_eq!(
        screen.player_control_at(point, SIZE),
        Some(PlayerAction::Remove(53))
    );
    let SubScreen::Configure {
        detail: Some(Ok(detail)),
        ..
    } = &mut screen.subscreen
    else {
        unreachable!()
    };
    detail.players.swap(1, 52);
    assert_eq!(screen.player_control_at(point, SIZE), None);
    assert!(screen.activate_player_focus().is_some());
    assert!(
        matches!(&screen.subscreen, SubScreen::Popup(super::super::Popup::Uninvite { player, .. }) if player_uuid(player) == Some(53))
    );
}

#[test]
fn injected_ops_reply_requires_repainting_before_activation() {
    use super::super::{Arc, Mutex, client::Ops};
    let mut screen = super::super::tests::loaded_players(8);
    draw(&screen);
    let [x, y, w, h] = screen.move_player_focus(KeyCode::ArrowDown, SIZE).unwrap();
    let point = (x + w / 2.0, y + h / 2.0);
    draw(&screen);
    assert_eq!(
        screen.player_control_at(point, SIZE),
        Some(PlayerAction::Op(1))
    );
    assert_eq!(screen.displayed_player_action(), Some(PlayerAction::Op(1)));
    let SubScreen::Configure { member_job, .. } = &mut screen.subscreen else {
        unreachable!()
    };
    *member_job = Some(Arc::new(Mutex::new(Some(Ok(Ops {
        ops: vec!["Player001".into()],
    })))));
    screen.poll();
    assert!(screen.activate_player_focus().is_none());
    assert_eq!(screen.player_control_at(point, SIZE), None);
    assert!(!screen.player_action_drawn(PlayerAction::Deop(1)));
    draw(&screen);
    assert_eq!(
        screen.displayed_player_action(),
        Some(PlayerAction::Deop(1))
    );
}
