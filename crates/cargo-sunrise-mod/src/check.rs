//! Package inspection and the release attestation policy for `cargo sunrise-mod check <file>`.
use crate::bounded::{self, Symlinks};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use sunrise_attestation::{Attested, PUBLIC_GOOD_TRUSTED_ROOT};
use sunrise_mod_package::Package;
use sunrise_mod_package::index::{self, Facts, Kind};
use sunrise_mod_package::manifest::{MANIFEST, MAX_USES};

pub struct Report {
    pub kind: Kind,
    pub facts: Facts,
    pub sha256: String,
    pub size: u64,
    pub signed: bool,
    attestation: Option<Attested>,
}

impl Report {
    pub fn json(&self) -> Value {
        let requires: Vec<Value> = self
            .facts
            .requires
            .iter()
            .map(|r| json!({"id": r.id, "version": r.version, "sha256": r.sha256}))
            .collect();
        let mut report = json!({
            "kind": self.kind.name(),
            "id": self.facts.id,
            "version": self.facts.version,
            "api": self.facts.api,
            "side": self.facts.side,
            "capabilities": self.facts.capabilities,
            "game": self.facts.game,
            "dependencies": self.facts.dependencies,
            "requires": requires,
            "sha256": self.sha256,
            "size": self.size,
            "signed": self.signed,
        });
        if let Some(attested) = &self.attestation {
            report["attestation"] = json!({
                "repository_id": attested.repository_id,
                "repository_name": attested.repository_name,
                "signer_workflow_ref": attested.signer_workflow_ref,
                "source_ref": attested.source_ref,
                "source_commit": attested.source_commit,
                "predicate_type": attested.predicate_type,
            });
        }
        report
    }

    pub fn text(&self) -> String {
        let f = &self.facts;
        let mut out = format!(
            "{} {} ({}",
            f.id.as_deref().unwrap_or("resource pack"),
            f.version.as_deref().unwrap_or(""),
            self.kind.name()
        );
        if let Some(side) = f.side {
            let side = serde_json::to_value(side).unwrap_or_default();
            out += &format!(", API {}, {}", f.api, side.as_str().unwrap_or("?"));
        } else if self.kind == Kind::Shader {
            out += &format!(", shader API {}", f.api);
        } else if !f.api.is_empty() {
            out += &format!(", format {}", f.api);
        }
        if let Some(game) = &f.game {
            out += &format!(", for {game}");
        }
        let signed = if self.signed {
            "signature verified"
        } else {
            "unsigned"
        };
        out += &format!(
            ")\n  {} bytes, SHA-256 {}, {signed}\n",
            self.size, self.sha256
        );
        if let Some(attested) = &self.attestation {
            out += &format!(
                "  repository: {} (ID {})\n  workflow: {}\n  commit: {}\n",
                attested.repository_name,
                attested.repository_id,
                attested.signer_workflow_ref,
                attested.source_commit
            );
        }
        for c in &f.capabilities {
            out += &format!("  may {}\n", c.describe());
        }
        for dep in &f.dependencies {
            let label = if dep.optional {
                "optional:"
            } else {
                "requires"
            };
            let side = match dep.side {
                Some(sunrise_mod_package::manifest::Side::Client) => " (client only)",
                Some(sunrise_mod_package::manifest::Side::Server) => " (dedicated server only)",
                _ => "",
            };
            out += &format!("  {label} {} {}{side}\n", dep.id, dep.version);
        }
        for r in &f.requires {
            out += &format!("  needs {} {} ({})\n", r.id, r.version, &r.sha256[..12]);
        }
        if self.kind == Kind::NativeMod {
            out += "  native source: unsandboxed; not executed by the registry check. Compile and test against the exact pinned Sunrise and Mixins revisions before review.\n";
        }
        if self.kind == Kind::Mod {
            out += "  no code: content only, nothing to run\n";
        }
        out
    }
}

/// The kind of a package, inspecting mod archives for their native manifest.
pub fn kind_of(path: &Path) -> Option<Kind> {
    match path.extension()?.to_str()? {
        "smod" if index::native::holds(path) => Some(Kind::NativeMod),
        "smod" => Some(Kind::Mod),
        "spack" => Some(Kind::Modpack),
        "zip" => Some(sunrise_mod_package::index::zip_kind(path)),
        "sshader" => Some(Kind::Shader),
        _ => None,
    }
}

/// Inspect a downloaded package and require an attestation for mods. A mod is read once into a
/// private copy, and that copy is hashed and verified before its contents are inspected: the facts
/// reported are those of the attested bytes. Whether a file is a mod is decided by its contents
/// too: a `.smod`, a mod `--kind`, or any archive with `sunrise-mod.toml` at its root.
pub fn check_release(
    path: &Path,
    kind: Option<Kind>,
    attestation: Option<&Path>,
    repository: Option<u64>,
) -> Result<Report, String> {
    let bytes =
        bounded::read_bytes(path, index::MAX_FILE, Symlinks::Follow).map_err(|e| e.to_string())?;
    let holds_manifest = zip::ZipArchive::new(std::io::Cursor::new(&bytes))
        .is_ok_and(|mut archive| archive.by_name(MANIFEST).is_ok());
    let mod_archive = holds_manifest || path.extension().is_some_and(|ext| ext == "smod");
    let as_mod = kind
        .as_ref()
        .is_some_and(|kind| matches!(kind, Kind::Mod | Kind::NativeMod));
    if !mod_archive && !as_mod {
        if attestation.is_some() || repository.is_some() {
            return Err("--attestation and --repository apply only to mod packages".into());
        }
        let kind = kind
            .or_else(|| kind_of(path))
            .ok_or("pass --kind mod|modpack|resourcepack: the extension doesn't say")?;
        return inspect(path, kind);
    }
    if kind.as_ref().is_some_and(|_| !as_mod) {
        return Err("a mod package must be checked as a mod or native-mod".into());
    }
    let default_sidecar;
    let sidecar = match attestation {
        Some(sidecar) => sidecar,
        None => {
            default_sidecar = path.with_added_extension("sigstore.json");
            &default_sidecar
        }
    };
    // The registry downloads a release's attestation within its bound for one release file.
    let bundle = bounded::read_bytes(sidecar, index::MAX_FILE, Symlinks::Follow).map_err(|e| {
        if e.is_not_found() {
            format!(
                "not signed: the release workflow attests each mod package; bundle not found: {}",
                sidecar.display()
            )
        } else {
            e.to_string()
        }
    })?;
    let attested = sunrise_attestation::verify(
        &bundle,
        Sha256::digest(&bytes).into(),
        PUBLIC_GOOD_TRUSTED_ROOT,
    )
    .map_err(|e| e.to_string())
    .and_then(|attested| require_release_signer(attested, repository))
    .map_err(|e| format!("{}: {e}", sidecar.display()))?;
    // A fixed name: the download's own name never becomes part of a path written here.
    let private = tempfile::tempdir().map_err(|e| format!("check: {e}"))?;
    let copy = private.path().join("package.smod");
    std::fs::write(&copy, &bytes).map_err(|e| format!("{}: {e}", copy.display()))?;
    let kind = kind.unwrap_or_else(|| {
        if index::native::holds(&copy) {
            Kind::NativeMod
        } else {
            Kind::Mod
        }
    });
    let mut report = inspect(&copy, kind)?;
    report.attestation = Some(attested);
    report.signed = true;
    Ok(report)
}

pub(crate) fn require_release_signer(
    attested: Attested,
    repository: Option<u64>,
) -> Result<Attested, String> {
    const WORKFLOW: &str = "sunrise-org/sunrise-mods/.github/workflows/release.yml";
    // The shared workflow's tags are `release-v<number>` (sunrise-mods registry/release-workflow.toml).
    const TAG_PREFIX: &str = "refs/tags/release-v";
    if let Some(expected) = repository
        && attested.repository_id != expected
    {
        return Err(format!(
            "expected repository ID {expected}, attestation names {} (ID {})",
            attested.repository_name, attested.repository_id
        ));
    }
    let allowed =
        attested
            .signer_workflow_ref
            .split_once('@')
            .is_some_and(|(workflow, git_ref)| {
                workflow == WORKFLOW
                    && git_ref.strip_prefix(TAG_PREFIX).is_some_and(|version| {
                        !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit())
                    })
            });
    if !allowed {
        return Err(format!(
            "expected workflow {WORKFLOW}@{TAG_PREFIX}<number>, attestation names {}",
            attested.signer_workflow_ref
        ));
    }
    Ok(attested)
}

/// Validate a package's contents, independent of signature or registry policy.
pub(crate) fn inspect(path: &Path, kind: Kind) -> Result<Report, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if size > index::MAX_FILE {
        return Err(format!("over {} MiB", index::MAX_FILE >> 20));
    }
    index::open(&kind)?;
    let facts = index::inspect(&kind, path)?;
    if kind == Kind::NativeMod {
        uses_list(path)?;
    }
    Ok(Report {
        kind,
        facts,
        sha256: index::sha256_file(path)?,
        size,
        signed: false,
        attestation: None,
    })
}

/// A native mod carries the list of what it uses, made for the build it pins: without it the mod
/// works on that one build only. A mod with rows no list can judge
/// ([`sunrise_native_build::game_uses::unjudged`]) is made for one build by nature and needs none.
fn uses_list(path: &Path) -> Result<(), String> {
    use sunrise_mod_package::mod_surface::uses::{USES_FILE, UsesTable};
    let manifest = Package::read(path)?.manifest;
    if sunrise_native_build::game_uses::unjudged(&manifest).is_some() {
        return Ok(());
    }
    let pin = manifest
        .native
        .ok_or_else(|| format!("no [native] table in {MANIFEST}"))?
        .sunrise;
    let repack = "Pack the mod with `cargo sunrise-mod build`, which writes the list; if it says it cannot read the mod's code, fix what it names first.";
    let Some(bytes) = Package::entry(path, USES_FILE, MAX_USES)? else {
        return Err(format!(
            "no {USES_FILE}: the package does not list what the mod uses, so it would work on game build {pin} only and stop at the next one. {repack}"
        ));
    };
    let list = UsesTable::read(&bytes).map_err(|e| format!("{USES_FILE}: {e}. {repack}"))?;
    if list.made_for != pin {
        return Err(format!(
            "{USES_FILE} was made for game build {}, and the mod pins {pin}, so the list says nothing about the pinned build. {repack}",
            list.made_for
        ));
    }
    Ok(())
}

/// Loads every listed `[[jar]]` from the local jar store, fetching a missing or changed jar from
/// Modrinth.
/// Returns `None` when the manifest lists no jars or when any jar cannot be fetched or read,
/// printing one note so `check` and `build` still succeed offline.
fn load_jar_layer(
    manifest: &sunrise_mod_package::Manifest,
) -> Option<sunrise_mod_package::content::JarLayer> {
    if manifest.jar.is_empty() {
        return None;
    }
    let store = crate::jar::default_store();
    let mut layer = sunrise_mod_package::content::JarLayer::new();
    for jar in &manifest.jar {
        let cached = jar.cache_path(&store);
        // A cached file is used only when its bytes still match the listed SHA-512, as `run`
        // checks before staging; otherwise it is fetched again, which replaces it.
        let verified = std::fs::File::open(&cached)
            .and_then(|file| sunrise_jar_fetch::hash_reader::<sha2::Sha512>(file, std::io::sink()))
            .is_ok_and(|hash| format!("{hash:x}") == jar.sha512());
        if !verified
            && let Err(error) = sunrise_jar_fetch::fetch(
                &crate::jar::ProxyHttp,
                crate::jar::MODRINTH,
                jar.sha512(),
                &cached,
            )
        {
            eprintln!(
                "note: could not fetch jar {} {} ({error}); its resources were not checked",
                jar.id(),
                jar.version()
            );
            return None;
        }
        if let Err(error) = layer.add_archive(&cached) {
            eprintln!(
                "note: could not read jar {} {} ({error}); its resources were not checked",
                jar.id(),
                jar.version()
            );
            return None;
        }
    }
    Some(layer)
}

/// The references among a project's content files ([`sunrise_mod_package::content::check_references`]):
/// warnings are printed, errors fail with one line each. When the manifest lists `[[jar]]`, each
/// jar is read from the local jar store (fetched from Modrinth if missing) so references resolve
/// against the mod and its jars together.
pub fn references(root: &Path, manifest: &sunrise_mod_package::Manifest) -> Result<(), String> {
    let content = sunrise_mod_package::Content::read(root, manifest)?;
    let jars = load_jar_layer(manifest);
    let findings =
        sunrise_mod_package::content::check_references(&content, manifest, jars.as_ref())?;
    for warning in findings
        .warnings
        .iter()
        .chain(&renamed_folders(&content.files))
    {
        eprintln!("warning: {warning}");
    }
    if findings.errors.is_empty() {
        Ok(())
    } else {
        Err(findings.errors.join("\nerror: "))
    }
}

/// Translates and validates every GLSL core shader program in a mod's `files`
/// (`assets/<namespace>/shaders/core/<path>.vsh` and `.fsh`) through `sunrise_mod_pipeline`
/// (the same code the game uses), returning how many programs were checked and each refusal as
/// `<file>[:<line>:<column>]: <message>`.
///
/// Includes resolve from the mod's own `files`, then from `resources` (prepared vanilla 26.3
/// resources, when available), and finally from Sunrise's interface-only stubs for `minecraft:*`
/// (§6 & §13 Q3: nothing from Mojang's files is shipped in the SDK). When a `.fsh` has no
/// matching `.vsh` beside it in `files`, `minecraft:shaders/core/position_tex_color.vsh` is used
/// as the default vertex stage (matching the common `.vertex_shader("minecraft:core/position_tex_color")`
/// case). Whether a pipeline is textured is in the mod's Rust code, so a program that references
/// `sampleSprite`, `pipeline_sprite.glsl`, `SpriteRect`, or `Sampler0` is checked as textured.
pub fn pipeline_shaders(
    files: &BTreeMap<String, Vec<u8>>,
    resources: Option<&Path>,
) -> (usize, Vec<String>) {
    let mut stems: BTreeMap<(String, String), (Option<String>, Option<String>)> = BTreeMap::new();
    let folder = sunrise_mod_pipeline::CORE_FOLDER;
    for name in files.keys() {
        let mut parts = name.splitn(3, '/');
        let (Some("assets"), Some(ns), Some(rest)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Some(after_folder) = rest.strip_prefix(folder).and_then(|r| r.strip_prefix('/')) else {
            continue;
        };
        if let Some(stem) = after_folder.strip_suffix(".vsh") {
            stems.entry((ns.to_owned(), stem.to_owned())).or_default().0 = Some(name.clone());
        } else if let Some(stem) = after_folder.strip_suffix(".fsh") {
            stems.entry((ns.to_owned(), stem.to_owned())).or_default().1 = Some(name.clone());
        }
    }

    let read_asset = |asset: &str| -> Result<Option<String>, String> {
        if let Some(bytes) = files.get(asset) {
            return std::str::from_utf8(bytes)
                .map(|s| Some(s.to_owned()))
                .map_err(|e| format!("{asset}: {e}"));
        }
        if let Some(dir) = resources {
            let path = dir.join(asset);
            match std::fs::read_to_string(&path) {
                Ok(text) => return Ok(Some(text)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        if let Some(rest) = asset.strip_prefix("assets/")
            && let Some((ns, path)) = rest.split_once('/')
            && let Some(stub) = sunrise_mod_pipeline::stubs::read(&format!("{ns}:{path}"))
        {
            return Ok(Some(stub.to_owned()));
        }
        Ok(None)
    };

    let include = |id: &str| -> Result<Option<String>, String> {
        let Some((ns, rest)) = id.split_once(':') else {
            return Ok(None);
        };
        read_asset(&format!("assets/{ns}/{rest}"))
    };

    let mut count = 0;
    let mut refused = Vec::new();
    for ((ns, stem), (vsh_file, fsh_file)) in stems {
        let Some(fsh_file) = fsh_file else {
            // Vertex-only files may be shared across pipelines via `.vertex_shader(...)`;
            // pair with a trivial fragment shader so the vertex stage is still validated.
            let vsh_file = vsh_file.expect("stem has at least one stage");
            count += 1;
            let vsh_text = match read_asset(&vsh_file) {
                Ok(Some(t)) => t,
                Ok(None) => {
                    refused.push(format!("{vsh_file}: not found"));
                    continue;
                }
                Err(e) => {
                    refused.push(e);
                    continue;
                }
            };
            let folder_part = stem.rfind('/').map_or("", |i| &stem[..=i]);
            let dir = format!("{ns}:{folder}/{folder_part}");
            let dummy_fsh =
                "#version 330\nout vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }\n";
            let textured = vsh_text.contains("Sampler0")
                || vsh_text.contains("SpriteRect")
                || vsh_text.contains("sampleSprite")
                || vsh_text.contains("pipeline_sprite.glsl");
            let sources = sunrise_mod_pipeline::ProgramSources {
                vertex_file: &vsh_file,
                vertex_dir: &dir,
                vertex: &vsh_text,
                fragment_file: &vsh_file,
                fragment_dir: &dir,
                fragment: dummy_fsh,
            };
            let options = sunrise_mod_pipeline::Options {
                textured,
                check_backends: true,
                ..Default::default()
            };
            if let Err(problems) = sunrise_mod_pipeline::translate(&sources, &options, &include) {
                refused.extend(problems.into_iter().map(|p| p.to_string()));
            }
            continue;
        };

        count += 1;
        let fsh_text = match read_asset(&fsh_file) {
            Ok(Some(t)) => t,
            Ok(None) => {
                refused.push(format!("{fsh_file}: not found"));
                continue;
            }
            Err(e) => {
                refused.push(e);
                continue;
            }
        };
        let (vertex_file, vertex_dir, vsh_text) = match vsh_file {
            Some(vsh_path) => {
                let text = match read_asset(&vsh_path) {
                    Ok(Some(t)) => t,
                    Ok(None) => {
                        refused.push(format!("{vsh_path}: not found"));
                        continue;
                    }
                    Err(e) => {
                        refused.push(e);
                        continue;
                    }
                };
                let folder_part = stem.rfind('/').map_or("", |i| &stem[..=i]);
                (vsh_path, format!("{ns}:{folder}/{folder_part}"), text)
            }
            None => {
                let default_vsh = "assets/minecraft/shaders/core/position_tex_color.vsh";
                let text = match read_asset(default_vsh) {
                    Ok(Some(t)) => t,
                    Ok(None) => {
                        refused.push(format!("{default_vsh}: not found"));
                        continue;
                    }
                    Err(e) => {
                        refused.push(e);
                        continue;
                    }
                };
                (
                    default_vsh.to_owned(),
                    "minecraft:shaders/core/".to_owned(),
                    text,
                )
            }
        };
        let folder_part = stem.rfind('/').map_or("", |i| &stem[..=i]);
        let fragment_dir = format!("{ns}:{folder}/{folder_part}");
        let textured = fsh_text.contains("sampleSprite")
            || fsh_text.contains("pipeline_sprite.glsl")
            || fsh_text.contains("SpriteRect")
            || fsh_text.contains("Sampler0")
            || vsh_text.contains("Sampler0");
        let sources = sunrise_mod_pipeline::ProgramSources {
            vertex_file: &vertex_file,
            vertex_dir: &vertex_dir,
            vertex: &vsh_text,
            fragment_file: &fsh_file,
            fragment_dir: &fragment_dir,
            fragment: &fsh_text,
        };
        let options = sunrise_mod_pipeline::Options {
            textured,
            check_backends: true,
            ..Default::default()
        };
        if let Err(problems) = sunrise_mod_pipeline::translate(&sources, &options, &include) {
            refused.extend(problems.into_iter().map(|p| p.to_string()));
        }
    }
    (count, refused)
}

/// One line per data folder that holds files under 26.2's name for a folder 26.3 reads under
/// another ([`sunrise_port_pack::upgrade::RENAMED_FOLDERS`]): the game reads nothing there, so
/// the files are silently left out.
fn renamed_folders(files: &[String]) -> Vec<String> {
    let mut found: BTreeMap<(&str, &str, &str), usize> = BTreeMap::new();
    for file in files {
        let Some((namespace, rest)) = file
            .strip_prefix("data/")
            .and_then(|rest| rest.split_once('/'))
        else {
            continue;
        };
        for (old, new) in sunrise_port_pack::upgrade::RENAMED_FOLDERS {
            if rest
                .strip_prefix(old)
                .is_some_and(|path| path.starts_with('/'))
            {
                *found.entry((namespace, old, new)).or_default() += 1;
            }
        }
    }
    found
        .into_iter()
        .map(|((namespace, old, new), count)| {
            format!(
                "data/{namespace}/{old}/: 26.3 does not read this folder, so its {count} file(s) are left out; 26.3 reads data/{namespace}/{new}/, in 26.3's format"
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_in_a_folder_26_3_renamed_are_named_once_per_folder() {
        let files = [
            "data/b/worldgen/configured_feature/arch.json",
            "data/b/worldgen/configured_feature/deep/rope.json",
            "data/b/worldgen/configured_carver/gap.json",
            "data/b/worldgen/feature/plank.json",
            "data/b/worldgen/configured_features/typo.json",
            "assets/b/worldgen/configured_feature/not_data.json",
        ]
        .map(String::from);
        assert_eq!(
            renamed_folders(&files),
            [
                "data/b/worldgen/configured_carver/: 26.3 does not read this folder, so its 1 file(s) are left out; 26.3 reads data/b/worldgen/carver/, in 26.3's format",
                "data/b/worldgen/configured_feature/: 26.3 does not read this folder, so its 2 file(s) are left out; 26.3 reads data/b/worldgen/feature/, in 26.3's format",
            ]
        );
    }

    #[test]
    fn pipeline_files_are_translated_as_the_game_does() {
        let vsh = "#version 330\n#include <minecraft:dynamictransforms.glsl>\n\
            #include <minecraft:projection.glsl>\n\
            in vec3 Position;\nin vec2 UV0;\nin vec4 Color;\n\
            out vec2 texCoord0;\nout vec4 vertexColor;\n\
            void main() { gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0); texCoord0 = UV0; vertexColor = Color; }\n";
        let glow_fsh = "#version 330\n#include <m:helper.glsl>\n\
            in vec2 texCoord0;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() { fragColor = tint(vertexColor); }\n";
        let sample_fsh = "#version 330\n#include <sunrise:pipeline_sprite.glsl>\n\
            in vec2 texCoord0;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() { fragColor = sampleSprite(texCoord0) * vertexColor; }\n";
        let broken_fsh =
            "#version 330\nout vec4 fragColor;\nvoid main() { fragColor = unknown_var; }\n";
        let files: BTreeMap<String, Vec<u8>> = [
            (
                "assets/m/shaders/include/helper.glsl",
                "vec4 tint(vec4 c) { return c * 0.5; }\n",
            ),
            ("assets/m/shaders/core/glow.vsh", vsh),
            ("assets/m/shaders/core/glow.fsh", glow_fsh),
            // Fragment-only program reusing default minecraft:core/position_tex_color.vsh stub:
            ("assets/m/shaders/core/deep/sample.fsh", sample_fsh),
            ("assets/m/shaders/core/broken.vsh", vsh),
            ("assets/m/shaders/core/broken.fsh", broken_fsh),
            // Old WGSL pipeline files and post shaders are not checked by the GLSL core rule.
            ("assets/m/shaders/pipeline/legacy.wgsl", "garbage"),
            ("assets/m/shaders/post/other.fsh", "garbage"),
            ("assets/m/lang/en_us.json", "{}"),
        ]
        .map(|(name, text)| (name.to_owned(), text.as_bytes().to_vec()))
        .into();
        let (count, refused) = pipeline_shaders(&files, None);
        assert_eq!(count, 3);
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert!(
            refused[0].starts_with("assets/m/shaders/core/broken.fsh:3:"),
            "{refused:?}"
        );
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sunrise-mod-package/tests/index")
            .join(name)
    }

    #[test]
    fn checks_each_kind_its_own_way() {
        let pack = fixture("cozy-1.0.0.spack");
        let report = inspect(&pack, kind_of(&pack).unwrap()).unwrap();
        assert_eq!(report.facts.requires[0].id, "coords_hud");
        assert!(
            report
                .text()
                .contains("cozy 1.0.0 (modpack, format 1, for 26.3)")
        );
        let smod = fixture("coords_hud-0.1.0.smod");
        let report = inspect(&smod, Kind::Mod).unwrap();
        assert!(report.text().contains("no code: content only"));
        let e = inspect(&pack, Kind::Shader).err().unwrap();
        assert!(e.contains("sunrise-shaders.toml"), "{e}");
        assert!(inspect(&pack, Kind::Mod).is_err());
    }

    #[test]
    fn unsigned_smod_is_refused_and_an_explicit_bundle_is_read() {
        let smod = fixture("coords_hud-0.1.0.smod");
        let missing = check_release(&smod, Some(Kind::Mod), None, None)
            .err()
            .unwrap();
        assert!(missing.ends_with("coords_hud-0.1.0.smod.sigstore.json"));

        let bundle = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sunrise-attestation/tests/fixtures/ruff-source.sigstore.json");
        let mismatch = check_release(&smod, None, Some(&bundle), None)
            .err()
            .unwrap();
        assert!(
            mismatch.contains("artifact hash does not match"),
            "{mismatch}"
        );
        // A file that isn't even an archive fails on its attestation, before anything parses it.
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.smod");
        std::fs::write(&junk, b"not an archive").unwrap();
        let refused = check_release(&junk, None, Some(&bundle), None)
            .err()
            .unwrap();
        assert!(
            refused.contains("artifact hash does not match"),
            "{refused}"
        );
    }

    #[test]
    fn a_mod_package_needs_its_attestation_whatever_its_name_or_kind() {
        let dir = tempfile::tempdir().unwrap();
        let renamed = dir.path().join("pack.zip");
        std::fs::copy(fixture("coords_hud-0.1.0.smod"), &renamed).unwrap();
        for kind in [None, Some(Kind::ResourcePack)] {
            assert!(check_release(&renamed, kind, None, None).is_err());
        }
        let missing = check_release(&renamed, None, None, None).err().unwrap();
        assert!(missing.ends_with("pack.zip.sigstore.json"), "{missing}");
        let smod = fixture("coords_hud-0.1.0.smod");
        assert!(check_release(&smod, Some(Kind::ResourcePack), None, None).is_err());
    }

    #[test]
    fn release_signer_must_be_the_shared_workflow_at_a_release_tag() {
        // Test policy on verified claims; the verifier crate exercises real signed bundles.
        let attested = Attested {
            repository_id: 123456,
            owner_id: 654321,
            repository_name: "author/repo".into(),
            signer_workflow_ref:
                "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/tags/release-v1".into(),
            source_ref: "refs/tags/v1.0.0".into(),
            source_commit: "cc67ad5440ad86ceddef5be9c7b1f931d5688a2a".into(),
            predicate_type: "https://slsa.dev/provenance/v1".into(),
        };
        assert_eq!(
            require_release_signer(attested.clone(), Some(123456)),
            Ok(attested.clone())
        );
        for signer_workflow_ref in [
            "author/repo/.github/workflows/release.yml@refs/tags/release-v1",
            "sunrise-org/sunrise-mods/.github/workflows/publish.yml@refs/tags/release-v1",
            "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/heads/release-v1",
            "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/tags/release-vnext",
            "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/tags/release-v1.2",
        ] {
            let error = require_release_signer(
                Attested {
                    signer_workflow_ref: signer_workflow_ref.into(),
                    ..attested.clone()
                },
                None,
            )
            .unwrap_err();
            assert!(error.contains(signer_workflow_ref), "{error}");
        }
        let repository = require_release_signer(attested, Some(1)).unwrap_err();
        assert_eq!(
            repository,
            "expected repository ID 1, attestation names author/repo (ID 123456)"
        );
    }

    #[test]
    fn dependencies_are_reported_as_required_or_optional() {
        let dependencies = json!([
            {"id": "creature", "version": "^0.1"},
            {"id": "jei", "version": "^0.5", "optional": true, "side": "client"}
        ]);
        let report = Report {
            kind: Kind::NativeMod,
            facts: Facts {
                id: Some("fireflies".into()),
                version: Some("1.0.0".into()),
                dependencies: serde_json::from_value(dependencies.clone()).unwrap(),
                ..Facts::default()
            },
            sha256: "0".repeat(64),
            size: 1,
            signed: false,
            attestation: None,
        };
        let text = report.text();
        for word in ["creature", "^0.1", "jei", "^0.5"] {
            assert!(text.contains(word), "{text}");
        }
        assert_eq!(report.json()["dependencies"], dependencies);
    }
}
