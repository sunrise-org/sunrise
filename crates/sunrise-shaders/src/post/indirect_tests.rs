//! `indirect.<pass>` (spec 3.2, line 398): a compute shader's work-group counts read from a
//! storage buffer, through the pack loader. Each malformed form is refused with a note naming
//! its key, and the compute shader it names is not run (decision D-10). The packs are original.

use super::*;

/// One unsuffixed and one suffixed compute shader of `composite`, both binding buffer 0.
const CSH: &str = "#version 430\nlayout(local_size_x=1) in;\n\
                   layout(std430, binding=0) buffer B { uint data[]; };\n\
                   void main() { data[0] = gl_NumWorkGroups.x; }\n";

/// The compute shaders of `composite` and the notes, loaded from `properties` through the
/// real reader (`read_properties`, with the pack's options) and `load_post`.
fn load(properties: &str, csh: &str) -> (Vec<(String, ComputeProgram)>, Vec<String>) {
    let zip = crate::pack::tests::zip(&[
        ("shaders/shaders.properties", properties),
        ("shaders/composite.csh", csh),
        ("shaders/composite_a.csh", CSH),
    ]);
    let pack = Pack::from_zip_bytes("indirect.zip", zip).unwrap();
    let mut macros = Macros::standard();
    let (properties, _) = crate::read_properties(&pack, &mut macros);
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    let compute = post
        .programs
        .into_iter()
        .find(|p| p.name == "composite")
        .map_or_else(Vec::new, |p| p.compute);
    (compute, post.notes)
}

fn indirect_of<'a>(
    compute: &'a [(String, ComputeProgram)],
    name: &str,
) -> Option<&'a ComputeProgram> {
    compute.iter().find(|(n, _)| n == name).map(|(_, c)| c)
}

#[test]
fn the_key_names_the_compute_shader_whose_counts_it_reads() {
    let (compute, notes) = load("bufferObject.0 = 64\nindirect.composite = 0 4\n", CSH);
    assert!(notes.is_empty(), "{notes:?}");
    let composite = indirect_of(&compute, "composite").expect("composite runs");
    assert_eq!(
        composite.indirect,
        Some(Indirect {
            buffer: 0,
            offset: 4
        })
    );
    assert_eq!(composite.work_groups, None);
    // `composite_a` has no key of its own: it keeps its direct dispatch.
    let suffixed = indirect_of(&compute, "composite_a").expect("composite_a runs");
    assert_eq!(suffixed.indirect, None);
}

#[test]
fn a_suffixed_compute_shader_takes_its_own_key() {
    let (compute, notes) = load("bufferObject.1 = 64\nindirect.composite_a = 1 8\n", CSH);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(indirect_of(&compute, "composite").unwrap().indirect, None);
    assert_eq!(
        indirect_of(&compute, "composite_a").unwrap().indirect,
        Some(Indirect {
            buffer: 1,
            offset: 8
        })
    );
}

/// Every malformed value: a note names the key, `composite` is not run, `composite_a` is.
#[test]
fn a_malformed_value_skips_the_shader_it_names_with_a_note() {
    let values = [
        "",
        "0",
        "0 4 8",
        "x 4",
        "0 x",
        "-1 4",
        "0 -4",
        "0 4.0",
        "0 0x4",
        "0 4294967296",
        // Not declared: only buffer 0 is.
        "3 0",
        // Not a multiple of 4.
        "0 2",
        "0 6",
        // The three counts end past the 64 bytes declared.
        "0 56",
        "0 4294967292",
    ];
    for value in values {
        let (compute, notes) = load(
            &format!("bufferObject.0 = 64\nindirect.composite = {value}\n"),
            CSH,
        );
        assert!(
            indirect_of(&compute, "composite").is_none(),
            "`{value}` ran; notes {notes:?}"
        );
        assert!(indirect_of(&compute, "composite_a").is_some(), "`{value}`");
        assert!(
            notes
                .iter()
                .any(|n| n.starts_with("`indirect.composite`: ")),
            "`{value}`: {notes:?}"
        );
        assert!(
            notes.iter().any(|n| n == "composite: skipped"),
            "`{value}`: {notes:?}"
        );
    }
}

/// The last three bytes the buffer declares hold the counts; one word further does not fit.
#[test]
fn the_counts_fit_a_fixed_buffer_up_to_its_last_byte() {
    let (compute, notes) = load("bufferObject.0 = 64\nindirect.composite = 0 52\n", CSH);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(
        indirect_of(&compute, "composite").unwrap().indirect,
        Some(Indirect {
            buffer: 0,
            offset: 52
        })
    );
    // A buffer started from a file is as big as it declares.
    let zip = crate::pack::tests::zip(&[
        (
            "shaders/shaders.properties",
            "bufferObject.0 = 16 seed.bin\nindirect.composite = 0 8\n",
        ),
        ("shaders/seed.bin", "abcd"),
        ("shaders/composite.csh", CSH),
    ]);
    let pack = Pack::from_zip_bytes("indirect.zip", zip).unwrap();
    let mut macros = Macros::standard();
    let (properties, _) = crate::read_properties(&pack, &mut macros);
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    assert!(
        post.notes
            .iter()
            .any(|n| n.starts_with("`indirect.composite`: ")),
        "{:?}",
        post.notes
    );
    assert!(post.programs.iter().all(|p| p.compute.is_empty()));
}

/// A buffer sized by the render size has no size at load: its range is checked as it runs.
#[test]
fn a_relative_buffer_is_taken_at_any_offset() {
    let (compute, notes) = load(
        "bufferObject.0 = 4 true 0.5 0.5\nindirect.composite = 0 4000\n",
        CSH,
    );
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(
        indirect_of(&compute, "composite").unwrap().indirect,
        Some(Indirect {
            buffer: 0,
            offset: 4000
        })
    );
}

/// The value's option names are the option's values, as for `image.*` (spec 1.5 item 2).
#[test]
fn an_option_in_the_value_is_its_value() {
    let csh = format!("#define COUNTS_AT 12 // [4 12]\n{CSH}");
    let (compute, notes) = load(
        "bufferObject.0 = 64\nindirect.composite = 0 COUNTS_AT\n",
        &csh,
    );
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(
        indirect_of(&compute, "composite").unwrap().indirect,
        Some(Indirect {
            buffer: 0,
            offset: 12
        })
    );
}

/// The key set twice: the last one counts, as for every key (`Properties::parse`).
#[test]
fn the_last_of_two_keys_counts() {
    let (compute, _) = load(
        "bufferObject.0 = 64\nindirect.composite = 0 4\nindirect.composite = 0 16\n",
        CSH,
    );
    assert_eq!(
        indirect_of(&compute, "composite").unwrap().indirect,
        Some(Indirect {
            buffer: 0,
            offset: 16
        })
    );
}

/// A `workGroups` over wgpu's limit is not what runs with the key, so it is not folded:
/// without the key it is, and the fold fixes `gl_NumWorkGroups`.
#[test]
fn a_shader_read_indirectly_keeps_its_work_group_built_ins() {
    let csh = "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups = ivec3(70000, 1, 1);\n\
               layout(std430, binding=0) buffer B { uint data[]; };\n\
               void main() { data[0] = gl_NumWorkGroups.x + gl_WorkGroupID.x; }\n";
    let built_ins = |c: &ComputeProgram| {
        c.module.entry_points[0]
            .function
            .arguments
            .iter()
            .filter_map(|a| a.binding.clone())
            .collect::<Vec<_>>()
    };
    let (compute, notes) = load("bufferObject.0 = 64\nindirect.composite = 0 0\n", csh);
    assert!(notes.is_empty(), "{notes:?}");
    let indirect = indirect_of(&compute, "composite").unwrap();
    assert_eq!(indirect.work_groups, None);
    let read = built_ins(indirect);
    for b in [naga::BuiltIn::NumWorkGroups, naga::BuiltIn::WorkGroupId] {
        assert!(
            read.contains(&naga::Binding::BuiltIn(b)),
            "{b:?} in {read:?}"
        );
    }
    let (compute, _) = load("bufferObject.0 = 64\n", csh);
    let direct = indirect_of(&compute, "composite").unwrap();
    assert!(
        direct.work_groups.is_some_and(|g| g != [70000, 1, 1]),
        "{:?}",
        direct.work_groups
    );
    assert!(
        !built_ins(direct).contains(&naga::Binding::BuiltIn(naga::BuiltIn::NumWorkGroups)),
        "the fold reads no `gl_NumWorkGroups`"
    );
}

/// The direct counts are not read for a shader read indirectly: a `workGroups` and a
/// `workGroupsRender` the direct path refuses (an expression, four numbers) don't skip it.
#[test]
fn a_shader_read_indirectly_does_not_check_its_direct_counts() {
    let csh = "#version 430\nlayout(local_size_x=1) in;\n\
               const ivec3 workGroups = ivec3(1 + 1, 1, 1);\n\
               const vec2 workGroupsRender = vec2(1.0 + 1.0 + 1.0, 1.0);\n\
               layout(std430, binding=0) buffer B { uint data[]; };\n\
               void main() { data[0] = gl_NumWorkGroups.x; }\n";
    let (compute, notes) = load("bufferObject.0 = 64\nindirect.composite = 0 4\n", csh);
    assert!(notes.is_empty(), "{notes:?}");
    let composite = indirect_of(&compute, "composite").expect("composite runs");
    assert_eq!(
        composite.indirect,
        Some(Indirect {
            buffer: 0,
            offset: 4
        })
    );
    // Without the key the same shader is refused for its counts.
    let (compute, notes) = load("bufferObject.0 = 64\n", csh);
    assert!(indirect_of(&compute, "composite").is_none(), "{notes:?}");
    assert!(notes.iter().any(|n| n == "composite: skipped"), "{notes:?}");
}
