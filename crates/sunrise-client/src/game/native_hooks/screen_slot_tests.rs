#![cfg(test)]

/// A data packet for an old window leaves the open mod screen alone.
#[test]
fn mod_screen_data_is_its_windows_only() {
    let mut s = super::OpenNativeScreen {
        slot_moves: Default::default(),
        layout_revision: 0,
        key: (0, 0, 0),
        window: 4,
        menu: "gtmod:menu".into(),
        block: Some("gtmod:machine".into()),
        screen: mod_api::Screen::default(),
        mod_type: None,
        data: Vec::new(),
        extra: None,
    };
    assert!(s.set_data(4, 1, 7));
    assert!(!s.set_data(3, 1, 99));
    assert_eq!(s.data, [0, 7]);
}

/// A slot move changes only a shown slot, to a finite position, takes
/// effect at the commit, and counts only what changed a position (a
/// repeat of the effective position, declared or moved, keeps the layout).
#[test]
fn slot_moves_refuse_what_the_screen_does_not_show() {
    let declared = [
        Some(mod_api::ScreenSlot {
            position: [8.0, 18.0],
            ..Default::default()
        }),
        None,
        Some(mod_api::ScreenSlot {
            position: [26.0, 18.0],
            ..Default::default()
        }),
    ];
    use mod_api::ScreenSlotRefusal::*;
    let moves = super::SlotMoves::default();
    // The current place again changes nothing, not even the count.
    assert_eq!(moves.set(&declared, 0, Some([8.0, 18.0])), Ok(()));
    assert_eq!(moves.count.get(), 0);
    assert_eq!(moves.set(&declared, 2, Some([100.5, -4.0])), Ok(()));
    assert_eq!(moves.count.get(), 1);
    // Pending until the build commits it; the read-back sees it at once.
    assert_eq!(moves.position(2, [26.0, 18.0]), Some([26.0, 18.0]));
    assert_eq!(moves.latest(&declared, 2), Some([100.5, -4.0]));
    moves.commit();
    assert_eq!(moves.position(2, [26.0, 18.0]), Some([100.5, -4.0]));
    assert_eq!(moves.position(0, [8.0, 18.0]), Some([8.0, 18.0]));
    // Refusals change nothing.
    for (slot, pos, refusal) in [
        (1, Some([0.0, 0.0]), HiddenAtOpen),
        (3, Some([0.0, 0.0]), NoSuchSlot),
        (usize::MAX, None, NoSuchSlot),
        (0, Some([f32::NAN, 0.0]), BadPosition),
        (0, Some([0.0, f32::INFINITY]), BadPosition),
    ] {
        assert_eq!(
            moves.set(&declared, slot, pos),
            Err(refusal),
            "{slot} {pos:?}"
        );
    }
    assert_eq!(moves.count.get(), 1);
    assert_eq!(moves.position(0, [8.0, 18.0]), Some([8.0, 18.0]));
    // Hiding a slot hidden at open is already so.
    assert_eq!(moves.set(&declared, 1, None), Ok(()));
    assert_eq!(moves.count.get(), 1);
    // Hidden, then shown again where the mod puts it.
    assert_eq!(moves.set(&declared, 2, None), Ok(()));
    assert_eq!(moves.count.get(), 2);
    assert_eq!(moves.set(&declared, 2, None), Ok(()));
    assert_eq!(moves.count.get(), 2);
    moves.commit();
    assert_eq!(moves.position(2, [26.0, 18.0]), None);
    assert_eq!(moves.set(&declared, 2, Some([26.0, 18.0])), Ok(()));
    moves.commit();
    assert_eq!(moves.position(2, [26.0, 18.0]), Some([26.0, 18.0]));
    assert_eq!(moves.count.get(), 3);
}
