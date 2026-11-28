//! Iris-path compatibility: the macros `IS_IRIS` and `IRIS_VERSION` (spec 1.5.1, line 168, and
//! OQ-2), which a pack tests to choose its Iris path. Sunrise defines them for a pack only while
//! its compatibility is on ([`Macros::set_identity`]): the player's switch says On, or Auto and
//! the pack asks for them ([`asked`]). Decision D-8 in docs/cleanroom/.

use crate::{Macros, Pack};

/// The identity macros Sunrise answers, in the order [`Macros::set_identity`] defines them.
pub const NAMES: [&str; 2] = ["IS_IRIS", "IRIS_VERSION"];

/// The value of `IRIS_VERSION` while compatibility is on. The spec gives the name and no value
/// (1.5.1, line 168); decision D-8 sets 11103, the value #6054's observations were made with, and
/// what it promises: that a pack testing it takes its Iris path, nothing more. It changes only by
/// a new dated decision.
pub const IRIS_VERSION: u32 = 11103;

impl Macros {
    /// Defines `IS_IRIS` as `1` and `IRIS_VERSION` as [`IRIS_VERSION`] when `on`; undefines both
    /// otherwise, so a set the caller reuses never carries another pack's answer.
    pub fn set_identity(&mut self, on: bool) {
        for name in NAMES {
            self.undef(name);
        }
        if on {
            self.define(NAMES[0], "1");
            self.define(NAMES[1], IRIS_VERSION.to_string());
        }
    }
}

/// Where `pack` (with the player's chosen options, `Pack::with_chosen`) asks for the identity
/// macros, as `file:line`: the first conditional directive whose evaluation looks one of them
/// up (`preprocess::first_lookup`), in the stage files the loader reads (`post::program_stages`,
/// the programs mods request, `mod_programs`, among them) of its base folder and of each folder
/// a dimension may load (`dimension_folders`), their includes with them. They are read as the
/// load's preprocessor reads them, without output, and the scan stops at the first lookup. They
/// are read with the macros the scan
/// gives every reader (decision D-8): the standard set and the pack's options and feature
/// answers, as `read_properties` leaves them over it, with no identity macro. `None`: the pack
/// never tests them in the branches those settings reach.
pub fn asked(pack: &Pack, mod_programs: &[String]) -> Option<String> {
    let mut macros = Macros::standard();
    macros.set_identity(false);
    crate::read_properties(pack, &mut macros);
    // Each file is read once for every stage that includes it.
    let mut texts = crate::preprocess::Texts::new();
    dimension_folders(pack).iter().find_map(|folder| {
        let stages = crate::post::program_stages(pack, folder, mod_programs);
        stages.iter().find_map(|path| {
            let (file, line) =
                crate::preprocess::first_lookup(pack, path, &macros, &NAMES, &mut texts)?;
            Some(format!("{file}:{line}"))
        })
    })
}

/// The folders a dimension's programs may be loaded from (`dimension_folder_named`): the base
/// folder, then those `dimension.properties` maps, or without it the `world<id>` folders of the
/// three legacy ids.
fn dimension_folders(pack: &Pack) -> Vec<String> {
    let mut folders = vec![String::new()];
    match pack.read_text("dimension.properties") {
        Some(text) => {
            let mappings = crate::Properties::parse(&text, &Macros::default());
            for entry in &mappings.entries {
                let folder = entry.key.strip_prefix("dimension.");
                let folder = folder.and_then(crate::pack::normalize);
                if let Some(folder) = folder.filter(|f| !f.is_empty() && !folders.contains(f)) {
                    folders.push(folder);
                }
            }
        }
        None => folders.extend(["world0", "world-1", "world1"].map(str::to_owned)),
    }
    folders
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::zip;

    /// A pack of `files` under `shaders/`, each vertex, fragment or geometry stage with the
    /// vertex and fragment stages of its program the loader needs (`post::program_stages`),
    /// empty where `files` lacks them.
    fn pack(files: &[(&str, &str)]) -> Pack {
        let mut files: Vec<(String, &str)> = files
            .iter()
            .map(|(p, t)| (format!("shaders/{p}"), *t))
            .collect();
        let programs: Vec<String> = (files.iter())
            .filter_map(|(p, _)| {
                let (program, ext) = p.rsplit_once('.')?;
                ["vsh", "fsh", "gsh"]
                    .contains(&ext)
                    .then(|| program.to_owned())
            })
            .collect();
        for program in programs {
            for ext in ["vsh", "fsh"] {
                let path = format!("{program}.{ext}");
                if !files.iter().any(|(p, _)| *p == path) {
                    files.push((path, "void main() {}\n"));
                }
            }
        }
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        Pack::from_zip_bytes("P.zip", zip(&files)).unwrap()
    }

    /// Where a final program whose fragment stage is `fsh` asks, if it does.
    fn asks(fsh: &str) -> Option<String> {
        asked(&pack(&[("final.fsh", fsh)]), &[])
    }

    /// Every conditional lookup of either name asks, at its line: `#ifdef`, `#ifndef`,
    /// `defined` with and without parentheses, a comparison, the name alone, an `#elif` that is
    /// reached, a macro that expands to it, and one that pastes it together.
    #[test]
    fn each_conditional_lookup_asks_at_its_line() {
        for (fsh, at) in [
            ("#ifdef IS_IRIS\n#endif\n", 1),
            ("\n#ifndef IRIS_VERSION // fallback\n#endif\n", 2),
            ("#if defined IS_IRIS\n#endif\n", 1),
            ("#if MC_VERSION > 0 && !defined(IRIS_VERSION)\n#endif\n", 1),
            ("#if IRIS_VERSION >= 10700\n#endif\n", 1),
            ("#if IS_IRIS\n#endif\n", 1),
            ("#ifdef NOPE\n#elif IRIS_VERSION > 1\n#endif\n", 2),
            ("#define CHECK defined IS_IRIS\n#if CHECK\n#endif\n", 2),
            ("#define V IRIS_VERSION\n#if V > 10000\n#endif\n", 2),
            ("#define CAT(a, b) a##b\n#if CAT(IS_, IRIS)\n#endif\n", 2),
        ] {
            assert_eq!(asks(fsh), Some(format!("final.fsh:{at}")), "{fsh}");
        }
    }

    /// A name that isn't looked up by a condition the preprocessor evaluates doesn't ask: in
    /// code, defined or undefined, in a comment, in a branch not taken, in an `#elif` after a
    /// taken branch, a `defined` of another macro whatever it holds, an argument a macro drops,
    /// or a longer name.
    #[test]
    fn names_no_evaluated_condition_looks_up_do_not_ask() {
        for fsh in [
            "float IS_IRIS = 1.0;\n",
            "#define IS_IRIS\n#undef IRIS_VERSION\n",
            "// #ifdef IS_IRIS\n/* #if IRIS_VERSION > 1 */\n",
            "#ifdef NOPE\n#ifdef IS_IRIS\n#endif\n#if IRIS_VERSION\n#endif\n#endif\n",
            "#ifdef MC_VERSION\n#elif defined IS_IRIS\n#endif\n",
            "#define FOO IS_IRIS\n#if defined FOO\n#endif\n#ifdef FOO\n#endif\n",
            "#ifdef IS_IRIS_X\n#endif\n#if MY_IRIS_VERSION\n#endif\n",
            "#define DROP(x)\n#if DROP(IS_IRIS) 1\n#endif\n",
        ] {
            assert_eq!(asks(fsh), None, "{fsh}");
        }
    }

    /// What the scan finds is the lookup the preprocessor makes, which decides the branch: the
    /// pasted name is looked up, and its branch is taken with the answer on and not without it;
    /// the dropped one isn't, and its branch is taken either way.
    #[test]
    fn the_lookup_found_is_the_one_that_decides_the_branch() {
        let pasted = "#define CAT(a, b) a##b\n#if CAT(IS_, IRIS)\nfloat taken;\n#endif\n";
        let dropped = "#define DROP(x)\n#if DROP(IS_IRIS) 1\nfloat taken;\n#endif\n";
        let taken = |fsh: &str, on: bool| {
            let mut m = Macros::standard();
            m.set_identity(on);
            crate::preprocess(&pack(&[("final.fsh", fsh)]), "final.fsh", &m)
                .code()
                .contains("float taken;")
        };
        assert_eq!(asks(pasted), Some("final.fsh:2".into()));
        assert_eq!((taken(pasted, false), taken(pasted, true)), (false, true));
        assert_eq!(asks(dropped), None);
        assert_eq!((taken(dropped, false), taken(dropped, true)), (true, true));
    }

    /// The scan reads the stage files the loader reads (`post::program_stages`) and only those:
    /// a program's stages, a compute program's `csh`, a mod's program, and a stage that is a
    /// link to a file elsewhere in a folder pack, which `Pack::exists` follows as the loader
    /// does; not a root file that is no program's (`unused.vsh`), nor a compute program's
    /// fragment stage, nor a mod program no mod requests.
    #[test]
    fn the_scan_reads_the_stages_the_loader_reads() {
        let check = "#ifdef IS_IRIS\n#endif\n";
        let found = |files: &[(&str, &str)], mods: &[String]| asked(&pack(files), mods);
        assert_eq!(
            found(&[("composite_b.csh", check)], &[]),
            Some("composite_b.csh:1".into())
        );
        assert_eq!(
            found(&[("deferred12.fsh", check)], &[]),
            Some("deferred12.fsh:1".into())
        );
        let mods = ["gbuffers_mod_glow".to_owned()];
        assert_eq!(
            found(&[("gbuffers_mod_glow.fsh", check)], &mods),
            Some("gbuffers_mod_glow.fsh:1".into())
        );
        for files in [
            &[("unused.vsh", check)][..],
            &[("composite_b.fsh", check)],
            &[("composite100.fsh", check)],
            &[("composite07.fsh", check)],
            &[("gbuffers_water_a.csh", check)],
            &[("gbuffers_mod_glow.fsh", check)],
        ] {
            assert_eq!(found(files, &[]), None, "{files:?}");
        }
        #[cfg(unix)]
        {
            let dir = crate::pack::tests::temp_dir("identity-link");
            let shaders = dir.join("shaders");
            std::fs::create_dir_all(shaders.join("lib")).unwrap();
            std::fs::write(shaders.join("lib/real.fsh"), check).unwrap();
            std::fs::write(shaders.join("final.vsh"), "void main() {}\n").unwrap();
            std::os::unix::fs::symlink(shaders.join("lib/real.fsh"), shaders.join("final.fsh"))
                .unwrap();
            let linked = asked(&Pack::open(&dir).unwrap(), &[]);
            std::fs::remove_dir_all(&dir).unwrap();
            assert_eq!(linked, Some("final.fsh:1".into()));
        }
    }

    /// The scan reads the includes, every stage the load reads, the dimension folders, and
    /// only those: a check in an included file, a vertex, geometry or compute stage, `world-1`,
    /// or a folder `dimension.properties` maps asks; one in a tessellation stage, an unused
    /// folder or a file no stage includes doesn't.
    #[test]
    fn the_scan_reads_the_stages_their_includes_and_the_dimension_folders() {
        let check = "#ifdef IS_IRIS\n#endif\n";
        let including = "#include \"/lib/check.glsl\"\n";
        let found = |files: &[(&str, &str)]| asked(&pack(files), &[]);
        assert_eq!(
            found(&[("composite.fsh", including), ("lib/check.glsl", check)]),
            Some("lib/check.glsl:1".into())
        );
        for stage in ["gbuffers_terrain.vsh", "shadow.gsh", "composite.csh"] {
            assert_eq!(found(&[(stage, check)]), Some(format!("{stage}:1")));
        }
        assert_eq!(
            found(&[("world-1/final.fsh", check)]),
            Some("world-1/final.fsh:1".into())
        );
        assert_eq!(
            found(&[
                (
                    "dimension.properties",
                    "dimension.nether_dir = the_nether\n"
                ),
                ("nether_dir/final.fsh", check),
            ]),
            Some("nether_dir/final.fsh:1".into())
        );
        for files in [
            &[("composite.tcs", check)][..],
            &[("lib/check.glsl", check)],
            &[("unused/final.fsh", check)],
            &[
                (
                    "dimension.properties",
                    "dimension.nether_dir = the_nether\n",
                ),
                ("world-1/final.fsh", check),
            ],
        ] {
            assert_eq!(found(files), None, "{files:?}");
        }
    }

    /// The scan picks a program's stages as the load does (`post::program_read`), each case one
    /// it once got wrong one way or the other. Read: a `setup` program's compute stages beside
    /// its vertex and fragment stages (`setup` is compute programs alone), the fragment stage of
    /// a shadow program with a tessellation stage (read for its buffer formats while the shadow
    /// pass is off), and `final`'s compute stage beside its drawn stages. Not read: a lone
    /// `final.csh` (`final` is never compute programs alone), `setup.vsh` and `setup.fsh`, and
    /// a composite program with a tessellation stage (skipped unread).
    #[test]
    fn the_scan_picks_a_programs_stages_as_the_load_does() {
        let check = "#ifdef IS_IRIS\n#endif\n";
        let found = |files: &[(&str, &str)]| asked(&pack(files), &[]);
        for (files, at) in [
            (&[("setup.fsh", ""), ("setup.csh", check)][..], "setup.csh"),
            (&[("setup.fsh", ""), ("setup_c.csh", check)], "setup_c.csh"),
            (&[("setup.csh", check)], "setup.csh"),
            (&[("shadow.fsh", check), ("shadow.tcs", "")], "shadow.fsh"),
            (&[("shadow.fsh", check), ("shadow.tes", "")], "shadow.fsh"),
            (&[("final.fsh", ""), ("final.csh", check)], "final.csh"),
        ] {
            assert_eq!(found(files), Some(format!("{at}:1")), "{files:?}");
        }
        for files in [
            &[("final.csh", check)][..],
            &[("final_a.csh", check)],
            &[("setup.fsh", check)],
            &[("setup.vsh", check)],
            &[("composite.fsh", check), ("composite.tcs", "")],
        ] {
            assert_eq!(found(files), None, "{files:?}");
        }
    }

    /// The scan keeps the conditional state the load's preprocessor keeps: a code line defines
    /// `__LINE__` for the directives after it, and a macro call over several lines gives the line
    /// it starts on. Each case asks where, and only where, the load with the answer on takes the
    /// branch behind the check.
    #[test]
    fn the_scan_sees_line_as_the_load_does() {
        for (fsh, at) in [
            (
                "float a;\n#ifdef __LINE__\n#ifdef IS_IRIS\nfloat taken;\n#endif\n#endif\n",
                Some(3),
            ),
            (
                "#ifdef __LINE__\n#ifdef IS_IRIS\nfloat taken;\n#endif\n#endif\n",
                None,
            ),
            (
                "#define F(a, b) a\nfloat x = F(1,\n2);\n#if __LINE__ == 2\n#ifdef IS_IRIS\n\
                 float taken;\n#endif\n#endif\n",
                Some(5),
            ),
            (
                "#define F(a, b) a\nfloat x = F(1,\n2);\n#if __LINE__ == 3\n#ifdef IS_IRIS\n\
                 float taken;\n#endif\n#endif\n",
                None,
            ),
        ] {
            assert_eq!(asks(fsh), at.map(|n| format!("final.fsh:{n}")), "{fsh}");
            let mut m = Macros::standard();
            m.set_identity(true);
            let code = crate::preprocess(&pack(&[("final.fsh", fsh)]), "final.fsh", &m);
            assert_eq!(code.code().contains("float taken;"), at.is_some(), "{fsh}");
        }
    }

    /// The scan sees the player's saved options and the feature answers, and no identity
    /// macro: a check only an option or a listed flag reaches asks once they are in place.
    #[test]
    fn the_scan_sees_saved_options_and_features_and_no_identity() {
        let files = [
            ("shaders.properties", "iris.features.optional = SSBO\n"),
            (
                "final.fsh",
                "//#define COLORED\n#ifdef COLORED\n#ifdef IRIS_FEATURE_SSBO\n\
                 #ifndef IS_IRIS\n#endif\n#endif\n#endif\n",
            ),
        ];
        let plain = pack(&files);
        assert_eq!(asked(&plain, &[]), None, "the option is off by default");
        let chosen = std::collections::BTreeMap::from([("COLORED".into(), "true".into())]);
        assert_eq!(
            asked(&plain.with_chosen(chosen), &[]),
            Some("final.fsh:4".into())
        );
        let unlisted = pack(&[("shaders.properties", ""), files[1]]);
        let chosen = std::collections::BTreeMap::from([("COLORED".into(), "true".into())]);
        assert_eq!(
            asked(&unlisted.with_chosen(chosen), &[]),
            None,
            "no SSBO answer"
        );
    }

    /// With the answer on, `shaders.properties` (read by `read_properties`, feature lists and
    /// all) and the programs `load_post` loads see both names with their values; with it off
    /// neither does.
    #[test]
    fn the_answer_reaches_shaders_properties_and_the_programs() {
        let vsh = "#version 330\nvoid main() { gl_Position = vec4(0.0); }\n";
        let fsh = "#version 330\nout vec4 c;\n#if defined IS_IRIS && IRIS_VERSION == 11103\n\
                   uniform float iris_path;\n#else\nconst float iris_path = 0.0;\n#endif\n\
                   void main() { c = vec4(iris_path); }\n";
        let pack = pack(&[
            (
                "shaders.properties",
                "#if IS_IRIS == 1\niris.features.optional = SSBO\nguarded = 1\n#endif\n",
            ),
            ("composite.vsh", vsh),
            ("composite.fsh", fsh),
        ]);
        let seen = |on: bool| {
            let mut m = Macros::standard();
            m.set_identity(on);
            let (properties, _) = crate::read_properties(&pack, &mut m);
            let post = crate::load_post(&pack, "", &properties, &m, &Default::default());
            let program = post.programs[0].program.as_ref().expect("composite loads");
            let uniform = (program.interface.uniforms.iter()).any(|u| u.name == "iris_path");
            (
                properties.get("guarded").is_some(),
                m.is_defined("IRIS_FEATURE_SSBO"),
                uniform,
            )
        };
        assert_eq!(seen(true), (true, true, true));
        assert_eq!(seen(false), (false, false, false));
    }

    /// A pack's line declaring an identity macro is no option, as a feature answer's isn't
    /// (`is_reserved`): it isn't listed, a saved choice doesn't rewrite it, and it is never
    /// defined for `shaders.properties`, so Off stays off there and On keeps 11103.
    #[test]
    fn a_packs_option_named_as_an_identity_macro_changes_no_answer() {
        let options = "#define IRIS_VERSION 7 // [7 8]\n//#define IS_IRIS\n";
        let pack = pack(&[
            (
                "shaders.properties",
                "#ifdef IRIS_VERSION\nany = 1\n#endif\n\
                 #if IRIS_VERSION == 11103\nanswered = 1\n#endif\n\
                 #ifdef IS_IRIS\nis = 1\n#endif\n",
            ),
            ("options.glsl", options),
            (
                "final.fsh",
                "#include \"/options.glsl\"\n#ifdef IS_IRIS\n#endif\n",
            ),
        ]);
        let listed = crate::options(&pack);
        assert!(
            listed.keys().all(|k| !NAMES.contains(&k.as_str())),
            "{listed:?}"
        );
        let chosen = std::collections::BTreeMap::from([
            ("IRIS_VERSION".to_owned(), "8".to_owned()),
            ("IS_IRIS".to_owned(), "true".to_owned()),
        ]);
        let pack = pack.with_chosen(chosen);
        assert_eq!(pack.read_text("options.glsl").as_deref(), Some(options));
        for on in [false, true] {
            let mut m = Macros::standard();
            m.set_identity(on);
            let (properties, _) = crate::read_properties(&pack, &mut m);
            let read = ["any", "answered", "is"].map(|k| properties.get(k).is_some());
            assert_eq!(read, [on; 3], "on: {on}");
            assert_eq!(m.get("IRIS_VERSION"), on.then_some("11103"));
        }
        // The scan's read has neither name from an option: the program's test still asks.
        assert_eq!(asked(&pack, &[]), Some("final.fsh:2".into()));
    }

    /// On defines both names, Off removes both, from a set that already held them too.
    #[test]
    fn set_identity_defines_both_or_neither() {
        let mut m = Macros::standard();
        m.set_identity(true);
        assert_eq!(m.get("IS_IRIS"), Some("1"));
        assert_eq!(m.get("IRIS_VERSION"), Some("11103"));
        m.set_identity(false);
        assert_eq!((m.get("IS_IRIS"), m.get("IRIS_VERSION")), (None, None));
        assert_eq!(m, Macros::standard());
    }
}
