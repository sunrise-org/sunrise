//! `bossBattle` (spec 5.2, decision D-12): the game's boss-bar kind reaches programs that declare
//! it and the pack's custom uniforms, as an `int`.

use super::initial_history_tests::test_frame;
use super::{FrameUniforms, NAMES, PackInputs, Value, index, index_in, zero};

fn boss_battle(u: &FrameUniforms) -> Value {
    u.values[index("bossBattle").expect("a built-in uniform")]
}

/// Each kind the game passes is the value, none included; a program of either kind finds the
/// name (so it gets no "shader defaults or zero" note) and reads it as `int` or `bool`, not as
/// a `float`.
#[test]
fn boss_battle_is_the_inputs_kind() {
    let f = test_frame();
    let mut u = FrameUniforms::default();
    for kind in [0, 1, 2, 3, 4] {
        let input = PackInputs {
            boss_battle: kind,
            ..Default::default()
        };
        u.update(&f, &input, [800, 450]);
        match boss_battle(&u) {
            Value::I(v) => assert_eq!(v, [kind, 0, 0]),
            Value::F(_) => panic!("bossBattle is an int"),
        }
    }
    assert!(index_in("bossBattle", false, false).is_some());
    assert!(index_in("bossBattle", true, false).is_some());
    let value = boss_battle(&u);
    assert!(value.read_as("bossBattle", "int"));
    assert!(value.read_as("bossBattle", "bool"));
    assert!(!value.read_as("bossBattle", "float"));
}

/// Custom uniforms see `bossBattle` as an `int` built-in: an `int` uniform of it alone keeps
/// it, and expressions compare it.
#[test]
fn custom_uniforms_read_boss_battle_as_an_int() {
    let properties = sunrise_shaders::Properties::parse(
        "uniform.int.kind = bossBattle
         uniform.float.wither = if(bossBattle == 3, 1.0, 0.0)
",
        &sunrise_shaders::Macros::standard(),
    );
    let mut custom = sunrise_shaders::Custom::parse(&properties);
    custom.eval(0.0, &zero);
    assert!(custom.notes.is_empty(), "{:?}", custom.notes);
    let f = test_frame();
    let mut u = FrameUniforms::default();
    let mut read = |kind: i32| -> [f32; 2] {
        let input = PackInputs {
            boss_battle: kind,
            ..Default::default()
        };
        u.update(&f, &input, [800, 450]);
        u.eval_custom(&mut custom);
        std::array::from_fn(|i| match u.values[NAMES.len() + i] {
            Value::F(v) => v[0],
            Value::I(_) => panic!("custom uniforms are stored as floats"),
        })
    };
    assert_eq!(read(3), [3.0, 1.0]);
    assert_eq!(read(2), [2.0, 0.0]);
    assert_eq!(read(0), [0.0, 0.0]);
}
