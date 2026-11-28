//! Original feature-negotiation control; no external shader implementation.
use std::io::{Cursor, Write};

use sunrise_shaders::{Macros, Pack, read_properties};

/// `shaders.properties` as the game reads it (`read_properties`, with the standard macros).
fn read(properties: &str) -> (sunrise_shaders::Properties, Vec<String>) {
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    archive
        .start_file(
            "shaders/shaders.properties",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    archive.write_all(properties.as_bytes()).unwrap();
    let pack = Pack::from_zip_bytes("culling.zip", archive.finish().unwrap().into_inner()).unwrap();
    read_properties(&pack, &mut Macros::standard())
}

#[test]
fn requested_reversed_culling_loads_and_packs_take_the_path_it_guards() {
    // A pack that asks for reversed culling, optional or required, loads without a note and
    // sees `IRIS_FEATURE_REVERSED_CULLING` (spec 3.2, decision D-7); one that doesn't ask keeps
    // its other value. `shadow.culling=reversed` written plainly still applies.
    let guarded = "#ifdef IRIS_FEATURE_REVERSED_CULLING\nshadow.culling=reversed\n\
                   #else\nshadow.culling=true\n#endif\n";
    for (request, culling) in [
        ("iris.features.optional=REVERSED_CULLING\n", "reversed"),
        ("iris.features.required=REVERSED_CULLING\n", "reversed"),
        ("", "true"),
    ] {
        let (properties, notes) = read(&format!("{request}{guarded}"));
        assert!(notes.is_empty(), "{request}: {notes:?}");
        assert_eq!(properties.get("shadow.culling"), Some(culling), "{request}");
    }
    let (properties, _) = read("shadow.culling=reversed\n");
    assert_eq!(properties.get("shadow.culling"), Some("reversed"));
}
