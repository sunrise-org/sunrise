//! The committed pack that asks for Sunrise's HDR extension (`tools/fixtures/shaderpacks/hdr_extension`, the frame
//! identity harness's `hdr_extension` sessions, and the example of `docs/shaders.md`): its request as the frame is
//! picked for it, behind its own option, and its programs translated both served and not, as the game loads them.

use std::collections::BTreeMap;
use std::path::Path;

use sunrise_shaders::{GameInputs, GbuffersInputs, Macros, Pack, Tonemap, load_post};

fn fixture(chosen: &[(&str, &str)]) -> Pack {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fixtures/shaderpacks/hdr_extension");
    let chosen: BTreeMap<String, String> = chosen
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    Pack::open(&root).unwrap().with_chosen(chosen)
}

/// The request read with the standard macros and the pack's options, as `picked_pack_shape` reads it.
fn request(pack: &Pack) -> Option<Tonemap> {
    let (properties, notes) = sunrise_shaders::read_properties(pack, &mut Macros::standard());
    assert!(notes.is_empty(), "{notes:?}");
    let (hdr, notes) = properties.sunrise_hdr();
    assert!(notes.is_empty(), "{notes:?}");
    hdr
}

#[test]
fn the_fixture_asks_for_the_curve_its_option_picks() {
    assert_eq!(request(&fixture(&[])), Some(Tonemap::Pack));
    assert_eq!(
        request(&fixture(&[("SUNRISE_CURVE", "true")])),
        Some(Tonemap::Sunrise)
    );
}

#[test]
fn the_fixture_s_final_translates_served_and_not() {
    for (chosen, served) in [
        (&[][..], None),
        (&[][..], Some(Tonemap::Pack)),
        (&[("SUNRISE_CURVE", "true")][..], Some(Tonemap::Sunrise)),
    ] {
        let pack = fixture(chosen);
        let mut macros = Macros::standard();
        macros.define_renderer(4);
        let (properties, notes) = sunrise_shaders::read_properties(&pack, &mut macros);
        assert!(notes.is_empty(), "{notes:?}");
        if let Some(tonemap) = served {
            macros.define_sunrise_hdr(tonemap);
        }
        let inputs = GameInputs {
            hdr: served,
            terrain: Some(GbuffersInputs::default()),
            ..Default::default()
        };
        let post = load_post(&pack, "", &properties, &macros, &inputs);
        assert!(post.notes.is_empty(), "{served:?}: {:?}", post.notes);
        assert_eq!(post.hdr, served);
        let names: Vec<&str> = post.programs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["final"], "{served:?}");
        let program = post.programs[0].program.as_ref().expect("final translated");
        assert!(program.notes.is_empty(), "{served:?}: {:?}", program.notes);
        let terrain = post.terrain.as_ref().and_then(|t| t.program.as_ref());
        assert!(
            terrain.is_some_and(|t| t.notes.is_empty()),
            "{served:?}: terrain"
        );
    }
}
