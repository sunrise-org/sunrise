//! `PER_BUFFER_BLENDING` answered like the other feature flags where the device draws it
//! (#6921; spec 3.2, decision D-7): a pack that lists it, in either list and any case, gets
//! `IRIS_FEATURE_PER_BUFFER_BLENDING` in `shaders.properties`' second read, its programs and its
//! id files; on a device without it the macro stays undefined and a required flag is noted.
//! Original fixtures, written here.
use super::read_properties;
use super::tests::{composite_uniforms, feature_pack};

const MACRO: &str = "IRIS_FEATURE_PER_BUFFER_BLENDING";

/// The feature flags the notes name, in order: tests check which flags are noted, not the
/// English of the note.
fn noted_flags(notes: &[String]) -> Vec<&str> {
    notes.iter().filter_map(|n| n.split('`').nth(1)).collect()
}

/// The standard macros with the device's answer: `Some(able)` as the renderer sets it, `None`
/// for the standard set alone (no renderer).
fn macros(able: Option<bool>) -> crate::Macros {
    let mut m = crate::Macros::standard();
    if let Some(able) = able {
        m.set_per_buffer_blending(able);
    }
    m
}

/// Listed in `iris.features.optional` in the pack's own case: on a device that draws it the
/// properties' second read, the programs and the id files see the macro under Sunrise's
/// spelling, beside the other flags' answers; without it, or without a renderer's answer, none
/// of them does and the other flags keep theirs.
#[test]
fn an_optional_flag_is_answered_by_the_device() {
    let features = [("seen_pbb", "PER_BUFFER_BLENDING"), ("seen_ssbo", "SSBO")];
    let text = "iris.features.optional = per_Buffer_blending SSBO\n\
                #ifdef IRIS_FEATURE_PER_BUFFER_BLENDING\nseen_properties = 1\n#endif\n\
                #ifdef IRIS_FEATURE_per_Buffer_blending\nseen_spelled = 1\n#endif\n";
    let pack = feature_pack(text, &features);
    let ids = "#ifdef IRIS_FEATURE_PER_BUFFER_BLENDING\nblock.7 = stone\n#endif\n";
    for (able, seen) in [(Some(true), true), (Some(false), false), (None, false)] {
        let mut m = macros(able);
        let (properties, notes) = read_properties(&pack, &mut m);
        assert!(notes.is_empty(), "{able:?}: {notes:?}");
        assert_eq!(m.get(MACRO), seen.then_some("1"), "{able:?}");
        assert!(
            !m.is_defined("IRIS_FEATURE_per_Buffer_blending"),
            "{able:?}"
        );
        assert_eq!(m.get("IRIS_FEATURE_SSBO"), Some("1"), "{able:?}");
        assert_eq!(
            properties.get("seen_properties"),
            seen.then_some("1"),
            "{able:?}"
        );
        assert_eq!(properties.get("seen_spelled"), None, "{able:?}");
        let expected: &[&str] = if seen {
            &["seen_pbb", "seen_ssbo"]
        } else {
            &["seen_ssbo"]
        };
        assert_eq!(
            composite_uniforms(&pack, &properties, &m),
            expected,
            "{able:?}"
        );
        let ids = crate::Properties::parse(ids, &m);
        assert_eq!(ids.get("block.7"), seen.then_some("stone"), "{able:?}");
    }
}

/// Listed in `iris.features.required` alone: on a device that draws it the pack gets the macro
/// and no note; without it, or without a renderer's answer, the macro stays undefined and the
/// flag is noted as written, as before.
#[test]
fn a_required_flag_is_answered_or_noted_by_the_device() {
    let features = [("seen_pbb", "PER_BUFFER_BLENDING")];
    let text = "iris.features.required = PER_BUFFER_BLENDING NOT_A_FEATURE\n";
    let pack = feature_pack(text, &features);
    for (able, seen) in [(Some(true), true), (Some(false), false), (None, false)] {
        let mut m = macros(able);
        let (properties, notes) = read_properties(&pack, &mut m);
        let expected: &[&str] = if seen {
            &["NOT_A_FEATURE"]
        } else {
            &["PER_BUFFER_BLENDING", "NOT_A_FEATURE"]
        };
        assert_eq!(noted_flags(&notes), expected, "{able:?}");
        assert_eq!(m.get(MACRO), seen.then_some("1"), "{able:?}");
        let uniforms: &[&str] = if seen { &["seen_pbb"] } else { &[] };
        assert_eq!(
            composite_uniforms(&pack, &properties, &m),
            uniforms,
            "{able:?}"
        );
    }
}

/// The answer comes in the same settled order as the other flags': a list only the macro
/// reaches is not read, one it would hide is.
#[test]
fn the_lists_are_settled_before_the_device_answer_is_defined() {
    let text = "iris.features.optional = PER_BUFFER_BLENDING\n\
                #ifdef IRIS_FEATURE_PER_BUFFER_BLENDING\n\
                iris.features.required = CUSTOM_IMAGES\nguarded = 1\n#endif\n\
                #ifndef IRIS_FEATURE_PER_BUFFER_BLENDING\n\
                iris.features.required = COMPUTE_SHADERS\n#endif\n";
    let pack = feature_pack(text, &[]);
    let mut m = macros(Some(true));
    let (properties, notes) = read_properties(&pack, &mut m);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(properties.get("guarded"), Some("1"));
    assert_eq!(m.get(MACRO), Some("1"));
    assert_eq!(m.get("IRIS_FEATURE_COMPUTE_SHADERS"), Some("1"));
    assert!(!m.is_defined("IRIS_FEATURE_CUSTOM_IMAGES"));
}

/// A macro set reused after a read that answered the flag holds no stale answer: the next
/// pack that doesn't list it, and the same pack on a device without it, read it undefined.
#[test]
fn a_reused_set_drops_the_answer() {
    let listing = feature_pack("iris.features.optional = PER_BUFFER_BLENDING\n", &[]);
    let unlisted = feature_pack("iris.features.optional = SSBO\n", &[]);
    let mut m = macros(Some(true));
    read_properties(&listing, &mut m);
    assert_eq!(m.get(MACRO), Some("1"));
    read_properties(&unlisted, &mut m);
    assert!(!m.is_defined(MACRO));
    read_properties(&listing, &mut m);
    m.set_per_buffer_blending(false);
    read_properties(&listing, &mut m);
    assert!(!m.is_defined(MACRO));
}
