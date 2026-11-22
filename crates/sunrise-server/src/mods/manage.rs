//! Operator commands for the shared server mod installer. Stdout is the result (or JSON);
//! confirmations, warnings and download progress go to stderr.
use super::{Lock, check_files, check_native, native, warn_takedowns, write_list};
use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};
use sunrise_mod_install::{
    Side,
    index::{Index, Kind, resolve},
    input::{ModId, ModRequest, Mods, Pack},
    lock::{BuildId, ReleaseTag},
    registry::{self, Https},
    selection::{self, Change, PackSource, Refresh, Target},
    transaction::{self, ServerFolder},
};
use sunrise_server_launch::{
    Install,
    config::{self, Config},
};

const GAME: &str = sunrise_platform::version::VERSION_ID;
const HELP: &str = "sunrise-server mods <add|remove|disable|enable|update|list|search|rollback|client-pack> [--json]";

pub enum Action {
    Add(Vec<ModRequest>, Option<PackSource>),
    Remove(Vec<ModId>),
    Disable(Vec<ModId>),
    Enable(Vec<ModId>),
    Update(Vec<ModId>),
    List,
    Search(String),
    ClientPack(PathBuf),
    Rollback,
}

struct Command {
    action: Action,
    json: bool,
    check: bool,
    yes: bool,
    trust_native: bool,
}

impl Command {
    fn parse(args: &[String]) -> Result<Self, String> {
        let Some((verb, rest)) = args.split_first() else {
            return Err("choose a mods command".into());
        };
        let mut cmd = Self {
            action: Action::List,
            json: false,
            check: false,
            yes: false,
            trust_native: false,
        };
        let mut operands = Vec::new();
        let mut pack = None;
        let mut out = None;
        let mut args = rest.iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--json" => cmd.json = true,
                "--check" if verb == "update" => cmd.check = true,
                "--trust-native-code" if matches!(verb.as_str(), "add" | "update" | "enable") => {
                    cmd.trust_native = true
                }
                "--yes" | "-y" if matches!(verb.as_str(), "update" | "rollback") => cmd.yes = true,
                "--out" if verb == "client-pack" && out.is_none() => {
                    out = Some(PathBuf::from(
                        args.next().ok_or("--out needs a new .spack path")?,
                    ));
                }
                "--pack" if verb == "add" && pack.is_none() => {
                    let raw = args
                        .next()
                        .ok_or("--pack needs a registry ID, .spack path or HTTPS URL")?;
                    pack = Some(PackSource::try_from(raw.clone())?);
                }
                option if option.starts_with('-') => {
                    return Err(format!("mods {verb}: unknown or repeated option {option}"));
                }
                _ => operands.push(arg.clone()),
            }
        }
        let ids = || {
            operands
                .iter()
                .cloned()
                .map(ModId::try_from)
                .collect::<Result<Vec<_>, _>>()
        };
        cmd.action = match verb.as_str() {
            "add" if pack.is_some() || !operands.is_empty() => Action::Add(
                operands
                    .into_iter()
                    .map(ModRequest::try_from)
                    .collect::<Result<_, _>>()?,
                pack,
            ),
            "remove" if !operands.is_empty() => Action::Remove(ids()?),
            "disable" if !operands.is_empty() => Action::Disable(ids()?),
            "enable" if !operands.is_empty() => Action::Enable(ids()?),
            "update" => Action::Update(ids()?),
            "list" if operands.is_empty() => Action::List,
            "search" if !operands.is_empty() => Action::Search(operands.join(" ")),
            "rollback" if operands.is_empty() => Action::Rollback,
            "client-pack" if operands.is_empty() => {
                Action::ClientPack(out.ok_or("client-pack needs --out FILE.spack")?)
            }
            _ => return Err(format!("mods {verb}: wrong arguments")),
        };
        Ok(cmd)
    }
}

pub(super) fn index_location() -> String {
    std::env::var("SUNRISE_MOD_INDEX")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| sunrise_addresses::MOD_INDEX.to_owned())
}

pub(super) fn current_target(
    index: &Index,
    config: &Config,
    old: Option<&Lock>,
) -> Result<Target<'static>, String> {
    let selected = config
        .engine
        .path
        .is_empty()
        .then(Install::find)
        .flatten()
        .and_then(|install| install.current());
    if let Some(tag) = selected {
        return release_target(index, &tag, old);
    }
    if let Some(old) = old {
        return Ok(Target {
            game: GAME,
            build: old.sunrise.clone(),
            release: old.release.clone(),
            side: Side::Server,
        });
    }
    Err("the installed Sunrise release is unknown\n  Choose a published build: sunrise-server update".into())
}

fn release_target(index: &Index, tag: &str, old: Option<&Lock>) -> Result<Target<'static>, String> {
    let build = index
        .releases
        .iter()
        .flatten()
        .find(|release| release.tag == tag)
        .map(|release| &release.commit)
        .or_else(|| {
            let nightly = index.nightly_build.as_ref()?;
            let stem = format!("nightly-{}-", nightly.date);
            let short = tag.strip_prefix(&stem)?;
            (short.len() >= 7 && nightly.commit.starts_with(short)).then_some(&nightly.commit)
        });
    if let Some(build) = build {
        return Ok(Target {
            game: GAME,
            build: BuildId::try_from(build.clone())?,
            release: Some(ReleaseTag::try_from(tag.to_owned())?),
            side: Side::Server,
        });
    }
    if let Some(old) = old
        && old
            .release
            .as_ref()
            .is_some_and(|release| release.to_string() == tag)
    {
        return Ok(Target {
            game: GAME,
            build: old.sunrise.clone(),
            release: old.release.clone(),
            side: Side::Server,
        });
    }
    Err(format!(
        "the mod registry doesn't know the build for Sunrise {tag}\n  Choose a published build: sunrise-server update"
    ))
}

/// Stage the existing selection for another Sunrise release. The caller stops the engine and
/// keeps the folder lease until `current` has switched (or both sides have been put back).
pub struct ReleaseChange {
    pub folder: ServerFolder,
    pub stage: transaction::Stage,
    pub text: String,
}

pub fn carry_release(
    dir: &Path,
    config: &Config,
    tag: &str,
) -> Result<Option<ReleaseChange>, String> {
    if config.mods.is_none() {
        return Ok(None);
    }
    if !config.engine.path.is_empty() {
        return Err("this server uses a custom engine; remove engine.path to carry mods between installed releases\n  Check the engine setting: sunrise-server config".into());
    }
    let folder = ServerFolder::open(dir)?;
    let root = folder.root();
    let text = fs::read_to_string(root.join(config::FILE)).map_err(|e| e.to_string())?;
    let cfg = Config::parse(&text)?;
    let old = read_optional_lock(root)?;
    check_ownership(root, &cfg, old.as_ref())?;
    let input = cfg
        .mods
        .ok_or("[mods] changed while preparing the release")?;
    let release = Some(ReleaseTag::try_from(tag.to_owned())?);
    let snapshot = root.join(transaction::RELEASE_PREVIOUS);
    if let Some(prior) = read_optional_lock(&snapshot)?
        && prior.release == release
    {
        if prior.mods.iter().any(|row| row.on && row.native.is_some()) && !input.trust_native_code {
            return Err("native code is not trusted; the previous release used native mods\n  Check the native code setting: sunrise-server config".into());
        }
        let stage = folder.stage_release_previous(&text)?;
        check_files(&prior, stage.path(), &prior.input).map_err(|e| e.to_string())?;
        selection::check_enabled(stage.path(), &prior)?;
        check_native(&prior, stage.path()).map_err(|e| e.to_string())?;
        return Ok(Some(ReleaseChange {
            folder,
            stage,
            text,
        }));
    }
    let fetch = Https::default();
    let index = registry::refresh(root, &index_location(), &fetch)?;
    let target = release_target(&index, tag, old.as_ref())?;
    let stage = folder.stage()?;
    let pack = input
        .pack
        .clone()
        .map(|pin| {
            selection::stage_pack(
                root,
                &stage,
                &fetch,
                &index,
                GAME,
                PackSource::Exact(pin),
                true,
            )
        })
        .transpose()?;
    let mut plan = selection::plan(
        &index,
        &target,
        input,
        pack.as_ref().map(|(_, pack)| &pack.manifest),
        old.as_ref(),
        Refresh::None,
        |p, v| native::refusal(&target, &index, p, v),
    )
    .map_err(|e| format!("{e}\n  Check for fitting mods: sunrise-server mods update --check"))?;
    show_plan(&selection::changes(old.as_ref(), Some(&plan.lock)), true);
    if plan.lock.mods.iter().any(|row| row.native.is_some()) && !plan.lock.input.trust_native_code {
        return Err("this release would install native code, which has not been allowed\n  Allow native code first: sunrise-server mods update --trust-native-code".into());
    }
    selection::stage_content(root, &stage, &fetch, &index, old.as_ref(), &plan, false)?;
    stage_runtime(root, &stage, &index, &fetch, &mut plan, old.as_ref())?;
    selection::stage_records(root, &stage, old.as_ref(), &plan.lock, &text)?;
    Ok(Some(ReleaseChange {
        folder,
        stage,
        text,
    }))
}

pub fn command(dir: &Path, args: &[String]) -> Result<(), String> {
    if args == ["--help"] || args == ["-h"] {
        println!(
            "{HELP}\n  add ID[@RANGE]… | --pack ID[@VERSION]|FILE.spack|URL\n  remove|disable|enable ID…\n  update [ID…] [--check] [--yes]\n  search TEXT…\n  list\n  rollback [--yes]\n  client-pack --out FILE.spack\n  add|enable|update may take --trust-native-code"
        );
        return Ok(());
    }
    let cmd = Command::parse(args)
        .map_err(|error| format!("{error}\n  See usage: sunrise-server mods --help"))?;
    if !dir.join(config::FILE).is_file() {
        return Err("no server.toml here: run it in your server's folder, or add --dir FOLDER\n  Set one up: sunrise-server init --eula".into());
    }
    (|| match cmd.action {
        Action::List => {
            let _lease = transaction::read_lease(dir)?;
            let root = dir
                .canonicalize()
                .map_err(|e| format!("{}: {e}", dir.display()))?;
            let lock = read_optional_lock(&root)?
                .ok_or("no managed mods here\n  Find a mod: sunrise-server mods search mods")?;
            match registry::load(&index_location(), &Https::default()) {
                Ok(index) => for warning in super::takedowns(&index, &lock) { eprintln!("{warning}"); },
                Err(_) => warn_takedowns(&root, &lock),
            }
            write_list(&lock, io::stdout().lock(), cmd.json).map_err(|e| e.to_string())
        }
        Action::ClientPack(out) => super::client_pack::write(dir, &out, cmd.json),
        Action::Search(query) => {
            let index = registry::load(&index_location(), &Https::default())?;
            let root = dir.canonicalize().map_err(|e| e.to_string())?;
            let cfg = Config::load(&root)?;
            let old = read_optional_lock(&root)?;
            let target = current_target(&index, &cfg, old.as_ref()).ok();
            search(&index, target.as_ref(), &query, cmd.json)
        }
        action => {
            // The engine may be writing persistent mod data, or holding SDK DLLs open.
            let _stopped = if cmd.check {
                None
            } else {
                Some(crate::supervisor::lock(dir).map_err(|_| {
                    "stop the server before changing its mods\n  Stop it: sunrise-server stop"
                        .to_owned()
                })?)
            };
            let folder = ServerFolder::open(dir)?;
            if matches!(action, Action::Rollback) && folder.recover()? {
                if cmd.json {
                    println!("{{\"recovered\":true}}");
                } else {
                    println!("Recovered the interrupted mod change.");
                }
                return Ok(());
            }
            let text = fs::read_to_string(folder.root().join(config::FILE))
                .map_err(|e| format!("{}: {e}", config::FILE))?;
            if matches!(action, Action::Rollback) && folder.root().join(transaction::RELEASE_PENDING).is_dir() {
                let current = Install::find().and_then(|install| install.current());
                let pending = read_optional_lock(&folder.root().join(transaction::RELEASE_PENDING))?;
                let live = read_optional_lock(folder.root())?;
                if live.as_ref().and_then(|lock| lock.release.as_ref()).is_some_and(|tag| Some(tag.to_string()) == current)
                    && pending.as_ref().and_then(|lock| lock.release.as_ref()).is_some_and(|tag| Some(tag.to_string()) != current) {
                    folder.finish_release(true)?;
                } else {
                    folder.undo_release(&text)?;
                }
                if cmd.json { println!("{{\"recovered\":true}}"); }
                else { println!("Recovered the interrupted release mod change. Verify: sunrise-server mods list"); }
                return Ok(());
            }
            let cfg = Config::parse(&text)?;
            make_change(
                &folder,
                &text,
                None,
                &cfg,
                action,
                cmd.json,
                cmd.check,
                cmd.yes,
                cmd.trust_native,
            )
        }
    })()
    .map_err(|error| with_next(error, "sunrise-server mods --help"))
}

fn with_next(error: String, command: &str) -> String {
    if error
        .lines()
        .last()
        .is_some_and(|line| line.starts_with("  ") && line.contains("sunrise-server "))
    {
        error
    } else {
        format!("{error}\n  Next: {command}")
    }
}

pub fn init(
    dir: &Path,
    cfg: &Config,
    input: Vec<ModRequest>,
    pack: Option<PackSource>,
    json: bool,
    yes: bool,
    trust_native: bool,
) -> Result<(), String> {
    let folder = ServerFolder::open(dir)?;
    let text = config::render(cfg);
    make_change(
        &folder,
        &text,
        Some(""),
        cfg,
        Action::Add(input, pack),
        json,
        false,
        yes,
        trust_native,
    )
    .map_err(|error| with_next(error, "sunrise-server init --eula"))
}

fn read_optional_lock(root: &Path) -> Result<Option<Lock>, String> {
    if root
        .join(super::LOCK)
        .try_exists()
        .map_err(|e| e.to_string())?
    {
        Lock::read(root).map(Some)
    } else {
        Ok(None)
    }
}

fn check_ownership(root: &Path, cfg: &Config, old: Option<&Lock>) -> Result<(), String> {
    if [
        "sunrise-mods",
        "sunrise-mods.txt",
        "mods.lock",
        ".mods-previous",
        ".mods-transaction",
        ".mods-pack.spack",
        transaction::NATIVE_DIR,
        ".mods-toolchains",
        transaction::RELEASE_PENDING,
        transaction::RELEASE_PREVIOUS,
    ]
    .iter()
    .any(|name| cfg.world.name.eq_ignore_ascii_case(name))
    {
        return Err("the world name is reserved for managed mod files; rename the world folder and set world.name before managing mods\n  Check the world setting: sunrise-server config".into());
    }
    if root
        .join(transaction::NATIVE_DIR)
        .symlink_metadata()
        .is_ok()
        && old
            .as_ref()
            .is_none_or(|lock| lock.mods.iter().all(|row| row.native.is_none()))
    {
        return Err("native/ isn't owned by the mod installer. Move that folder out of this server before managing its mods.\n  Then: sunrise-server mods update".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn make_change(
    folder: &ServerFolder,
    text: &str,
    expect: Option<&str>,
    cfg: &Config,
    action: Action,
    json: bool,
    check: bool,
    yes: bool,
    trust_native: bool,
) -> Result<(), String> {
    let root = folder.root();
    let old = read_optional_lock(root)?;
    check_ownership(root, cfg, old.as_ref())?;
    if matches!(action, Action::Rollback) {
        let prior = folder.previous_lock()?;
        if let Some(prior) = &prior {
            let path = root.join(transaction::PREVIOUS_DIR);
            if (prior.release != old.as_ref().and_then(|lock| lock.release.clone()))
                && old.is_some()
            {
                return Err("the previous mod selection belongs to a different Sunrise release\n  Roll back the game and its mods together: sunrise-server rollback".into());
            }
            if prior.mods.iter().any(|row| row.on && row.native.is_some())
                && !cfg.mods.as_ref().is_some_and(|mods| mods.trust_native_code)
            {
                return Err("the previous selection needs native code, which is not trusted\n  Install native mods explicitly: sunrise-server mods add ID --trust-native-code".into());
            }
            check_files(prior, &path, &prior.input).map_err(|e| e.to_string())?;
            check_native(prior, &path).map_err(|e| e.to_string())?;
            selection::check_enabled(&path, prior)?;
        }
        let changes = selection::changes(old.as_ref(), prior.as_ref());
        show_plan(&changes, json);
        if yes || confirm("Roll back?", false)? {
            folder.rollback(text)?;
            restored(json);
        } else if json {
            println!("{{\"changed\":false}}");
        }
        return Ok(());
    }
    let fetch = Https::default();
    let index = registry::refresh(root, &index_location(), &fetch).map_err(|e| {
        format!("{e}\n  Check the registry again: sunrise-server mods update --check")
    })?;
    let target = current_target(&index, cfg, old.as_ref())?;
    let mut input = cfg.mods.clone().unwrap_or_default();
    let already_trusted = input.trust_native_code;
    input.trust_native_code |= trust_native;
    let mut source = input.pack.clone().map(PackSource::Exact);
    let mut pack_changed = false;
    let mut refresh = Refresh::None;
    let mut update = false;
    let targeted;
    match action {
        Action::Add(requests, pack) => {
            for request in requests {
                input.list.retain(|old| old.id != request.id);
                input.list.push(request);
            }
            if pack.is_some() {
                source = pack;
                pack_changed = true;
            }
        }
        Action::Remove(ids) => {
            change_ids(&index, &mut input, old.as_ref(), &ids, Edit::Remove)?;
        }
        Action::Disable(ids) => {
            change_ids(&index, &mut input, old.as_ref(), &ids, Edit::Disable)?;
        }
        Action::Enable(ids) => {
            change_ids(&index, &mut input, old.as_ref(), &ids, Edit::Enable)?;
        }
        Action::Update(ids) => {
            update = true;
            if let Some(Pack::Registry { id, .. }) = &input.pack
                && (ids.is_empty() || ids.contains(id))
            {
                source = Some(PackSource::Newest(id.clone()));
            }
            for id in &ids {
                let installed = old
                    .as_ref()
                    .is_some_and(|old| old.mods.iter().any(|row| row.id == *id));
                let pack =
                    matches!(&input.pack, Some(Pack::Registry { id: pack, .. }) if pack == id);
                if !installed && !pack {
                    return Err(format!(
                        "{id} isn't installed\n  Add it: sunrise-server mods add {id}"
                    ));
                }
            }
            offer_takedowns(&index, &mut input, old.as_ref(), &ids, check, yes, json)?;
            targeted = ids;
            refresh = if targeted.is_empty() {
                Refresh::All
            } else {
                Refresh::Only(&targeted)
            };
        }
        Action::List | Action::Search(_) | Action::ClientPack(_) | Action::Rollback => {
            unreachable!("read commands do not stage an install")
        }
    }
    let stage = folder.stage()?;
    let retry_pack = source.as_ref().map(|source| match source {
        PackSource::Newest(id) => id.to_string(),
        PackSource::Exact(pack) => String::from(pack.clone()),
        PackSource::Url(url) => url.to_string(),
    });
    let mut pack = None;
    if let Some(source) = source {
        let (pin, contents) =
            selection::stage_pack(root, &stage, &fetch, &index, GAME, source, !pack_changed)?;
        pack_changed |= Some(&pin) != input.pack.as_ref();
        input.pack = Some(pin);
        pack = Some(contents);
    }
    let mut plan = selection::plan(
        &index,
        &target,
        input,
        pack.as_ref().map(|p| &p.manifest),
        old.as_ref(),
        refresh,
        |p, v| native::refusal(&target, &index, p, v),
    )?;
    let mut rejected_native = None;
    if !check {
        let native_rows: Vec<_> = plan
            .lock
            .mods
            .iter()
            .zip(&plan.artifacts)
            .filter(|(row, _)| {
                row.native.is_some()
                    && (row.on
                        || old.as_ref().is_none_or(|old| {
                            !old.mods
                                .iter()
                                .any(|prior| prior.id == row.id && prior.sha256 == row.sha256)
                        }))
            })
            .collect();
        if !native_rows.is_empty() {
            if !already_trusted
                || native_rows.iter().any(|(row, _)| {
                    old.as_ref()
                        .is_none_or(|old| !old.mods.iter().any(|prior| prior.id == row.id))
                })
            {
                eprintln!("Native mods run unsandboxed with this server account's rights:");
                for (row, artifact) in &native_rows {
                    let source = &artifact.project.source;
                    eprintln!(
                        "  {} {}  {}",
                        row.id,
                        row.version,
                        if source.is_empty() {
                            row.source_url()
                        } else {
                            source
                        }
                    );
                }
            }
            if !plan.lock.input.trust_native_code {
                if !yes
                    && !json
                    && io::stdin().is_terminal()
                    && confirm("Allow native code on this server?", false)?
                {
                    plan.lock.input.trust_native_code = true;
                } else {
                    let ids = native_rows
                        .iter()
                        .map(|(row, _)| row.id.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    let requested = plan
                        .lock
                        .input
                        .list
                        .iter()
                        .map(|row| String::from(row.clone()))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let pack_arg = retry_pack
                        .as_ref()
                        .map_or_else(String::new, |pack| format!(" --pack {pack:?}"));
                    let error = format!(
                        "{ids}: native code was not allowed\n  Allow it: sunrise-server mods add {requested}{pack_arg} --trust-native-code"
                    );
                    if expect.is_none() {
                        return Err(error);
                    }
                    plan = content_only(&index, &target, &plan)?;
                    rejected_native = Some(error);
                }
            }
        }
    }
    let changes = selection::changes(old.as_ref(), Some(&plan.lock));
    let modifies = !changes.is_empty()
        || old
            .as_ref()
            .is_none_or(|old| old.input != plan.lock.input || old.sunrise != plan.lock.sunrise);
    show_plan(&changes, json);
    if !plan.skipped_pack_members.is_empty() {
        eprintln!(
            "Skipping client-only pack members and their overrides: {}",
            plan.skipped_pack_members.join(", ")
        );
    }
    if check {
        if json {
            println!(
                "{}",
                serde_json::json!({"changed": modifies, "plan": changes})
            );
        } else if !modifies {
            println!("The mod selection is up to date.");
        }
        return Ok(());
    }
    let healthy = old.as_ref().is_some_and(|old| {
        check_files(old, root, &old.input).is_ok()
            && check_native(old, root).is_ok()
            && selection::check_enabled(root, old).is_ok()
    });
    let withdrawn_binary = old.as_ref().is_some_and(|lock| lock.mods.iter().any(|row| matches!(&row.native,
        Some(super::Native::Prebuilt(hash)) if index.revoked(row.id.as_str(), hash.as_str()).is_some())));
    if !modifies && healthy && !withdrawn_binary {
        if json {
            println!("{{\"changed\":false}}");
        } else {
            println!("The mod selection is up to date.");
        }
        return Ok(());
    }
    if update && !yes && !json && !confirm("Apply this mod update?", true)? {
        println!("No changes made.");
        return Ok(());
    }
    eprintln!("Verifying {} mod files…", plan.artifacts.len());
    selection::stage_content(
        root,
        &stage,
        &fetch,
        &index,
        old.as_ref(),
        &plan,
        pack_changed,
    )?;
    if rejected_native.is_some() && stage.path().join(transaction::PACK_FILE).is_file() {
        fs::remove_file(stage.path().join(transaction::PACK_FILE)).map_err(|e| e.to_string())?;
    }
    stage_runtime(root, &stage, &index, &fetch, &mut plan, old.as_ref())?;
    selection::stage_records(root, &stage, old.as_ref(), &plan.lock, text)?;
    folder.commit(stage, expect.unwrap_or(text))?;
    if let Some(error) = rejected_native {
        return Err(error);
    }
    if json {
        println!(
            "{}",
            serde_json::json!({"changed": true, "plan": selection::changes(old.as_ref(), Some(&plan.lock)), "lock": plan.lock})
        );
    } else {
        println!("Done. Start to use it: sunrise-server start\nUndo: sunrise-server mods rollback");
    }
    Ok(())
}

// Partial init does not leave a pack pin that would reinstall the declined native mods on the
// next update. Keep the independent content roots, including the pack's eligible content members.
fn content_only<'a>(
    index: &'a Index,
    target: &Target<'_>,
    plan: &selection::Plan<'_>,
) -> Result<selection::Plan<'a>, String> {
    let mut omitted: std::collections::BTreeSet<_> = plan
        .lock
        .mods
        .iter()
        .filter(|row| row.native.is_some())
        .map(|row| row.id.as_str())
        .collect();
    loop {
        let n = omitted.len();
        for a in &plan.artifacts {
            if a.version
                .dependencies
                .iter()
                .flatten()
                .any(|dep| !dep.optional && omitted.contains(dep.id.as_str()))
            {
                omitted.insert(a.project.id.as_str());
            }
        }
        if omitted.len() == n {
            break;
        }
    }
    let mut input = plan.lock.input.clone();
    input.pack = None;
    input.list.retain(|row| !omitted.contains(row.id.as_str()));
    for row in plan
        .lock
        .mods
        .iter()
        .filter(|row| row.reason == super::Reason::Pack && !omitted.contains(row.id.as_str()))
    {
        if !input.list.iter().any(|r| r.id == row.id) {
            input.list.push(ModRequest::try_from(format!(
                "{}@={}",
                row.id, row.version
            ))?);
        }
    }
    selection::plan(
        index,
        target,
        input,
        None,
        Some(&plan.lock),
        Refresh::None,
        |p, _| Some(format!("{} needs native-code consent", p.id)),
    )
}

fn stage_runtime(
    root: &Path,
    stage: &transaction::Stage,
    index: &Index,
    fetch: &Https,
    plan: &mut selection::Plan<'_>,
    old: Option<&Lock>,
) -> Result<(), String> {
    let identities = |lock: &Lock| {
        lock.mods
            .iter()
            .filter(|row| row.on && row.native.is_some())
            .map(|row| (row.id.clone(), row.sha256.clone()))
            .collect::<Vec<_>>()
    };
    let active = identities(&plan.lock);
    if !active.is_empty() && !plan.lock.input.trust_native_code {
        return Err("native code is not trusted\n  Allow it: sunrise-server mods update --trust-native-code".into());
    }
    if let Some(old) = old.filter(|old| {
        !active.is_empty()
            && old.sunrise == plan.lock.sunrise
            && identities(old) == active
            && old.mods.iter().all(|row| match &row.native {
                Some(super::Native::Prebuilt(hash)) => {
                    index.revoked(row.id.as_str(), hash.as_str()).is_none()
                }
                _ => true,
            })
            && check_native(old, root).is_ok()
    }) {
        plan.lock.runtime = old.runtime.clone();
        for row in &mut plan.lock.mods {
            if row.on && row.native.is_some() {
                row.native = old
                    .mods
                    .iter()
                    .find(|prior| prior.id == row.id)
                    .and_then(|prior| prior.native.clone());
            }
        }
        for file in old.runtime.keys() {
            let to = stage.path().join(file.as_path());
            fs::create_dir_all(to.parent().expect("locked runtime file has a folder"))
                .map_err(|e| e.to_string())?;
            fs::copy(root.join(file.as_path()), to).map_err(|e| e.to_string())?;
        }
    } else {
        native::stage(root, stage, index, fetch, plan)?;
    }
    if let Some(program) = check_native(&plan.lock, stage.path()).map_err(|e| e.to_string())? {
        crate::engine::check_starts(&program)?;
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    Remove,
    Disable,
    Enable,
}

fn change_ids(
    index: &Index,
    input: &mut Mods,
    old: Option<&Lock>,
    ids: &[ModId],
    edit: Edit,
) -> Result<(), String> {
    let old = old.ok_or("no managed mods here\n  Find a mod: sunrise-server mods search mods")?;
    let installed = old
        .mods
        .iter()
        .filter(|row| edit == Edit::Remove || row.on)
        .filter_map(|row| {
            let version = index
                .project(row.id.as_str())?
                .versions
                .iter()
                .find(|v| v.file.sha256 == row.sha256.as_str())?;
            Some(resolve::InstalledMod {
                id: row.id.to_string(),
                version: version.version.clone(),
                sha256: Some(version.file.sha256.clone()),
            })
        })
        .collect::<Vec<_>>();
    for id in ids {
        let row = old.mods.iter().find(|row| row.id == *id).ok_or_else(|| {
            format!("{id} isn't installed\n  Add it: sunrise-server mods add {id}")
        })?;
        if edit == Edit::Remove && row.reason == super::Reason::Pack {
            return Err(format!(
                "{id} comes with the pack\n  Turn it off: sunrise-server mods disable {id}"
            ));
        }
        if edit != Edit::Enable {
            let by = resolve::removal_dependents(index, &installed, id.as_str(), Side::Server)?;
            let remaining: Vec<_> = by
                .iter()
                .filter(|owner| !ids.iter().any(|target| target.as_str() == owner.id))
                .map(|owner| owner.id.as_str())
                .collect();
            if !remaining.is_empty() {
                let verb = if edit == Edit::Remove {
                    "remove"
                } else {
                    "disable"
                };
                return Err(format!(
                    "{} needs {id}\n  Change them together: sunrise-server mods {verb} {} {id}",
                    remaining.join(", "),
                    remaining.join(" ")
                ));
            }
        }
        match edit {
            Edit::Remove => {
                input.list.retain(|request| request.id != *id);
                input.off.retain(|off| off != id);
            }
            Edit::Disable if !input.off.contains(id) => input.off.push(id.clone()),
            Edit::Enable => input.off.retain(|off| off != id),
            Edit::Disable => {}
        }
    }
    Ok(())
}

fn offer_takedowns(
    index: &Index,
    input: &mut Mods,
    old: Option<&Lock>,
    ids: &[ModId],
    check: bool,
    yes: bool,
    json: bool,
) -> Result<(), String> {
    for row in old.into_iter().flat_map(|lock| &lock.mods) {
        if !ids.is_empty() && !ids.contains(&row.id) {
            continue;
        }
        let Some(revoked) = index.revoked(row.id.as_str(), row.sha256.as_str()) else {
            continue;
        };
        eprintln!(
            "{} {} was taken down ({}). Remove it: sunrise-server mods remove {}",
            row.id, row.version, revoked.reason, row.id
        );
        if !check
            && !yes
            && !json
            && io::stdin().is_terminal()
            && row.reason == super::Reason::Requested
            && confirm(&format!("Remove {}?", row.id), false)?
        {
            change_ids(
                index,
                input,
                old,
                std::slice::from_ref(&row.id),
                Edit::Remove,
            )?;
        }
    }
    Ok(())
}

fn confirm(question: &str, default: bool) -> Result<bool, String> {
    if !io::stdin().is_terminal() {
        return Ok(true);
    }
    eprint!("{question} [{}] ", if default { "Y/n" } else { "y/N" });
    io::stderr().flush().map_err(|e| e.to_string())?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let answer = line.trim();
    Ok((answer.is_empty() && default)
        || answer.eq_ignore_ascii_case("y")
        || answer.eq_ignore_ascii_case("yes"))
}

fn show_plan(changes: &[Change<'_>], json: bool) {
    let out = |line: &str| {
        if json {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    };
    if changes.is_empty() {
        out("No version or on/off changes.");
    }
    for change in changes {
        let status = if change.on == Some(false) {
            " (off)"
        } else {
            ""
        };
        out(&match (change.from, change.to) {
            (None, Some(to)) => format!("  {}: install {to}{status}", change.id),
            (Some(from), None) => format!("  {}: remove {from}", change.id),
            (Some(from), Some(to)) if from != to => {
                format!("  {}: {from} → {to}{status}", change.id)
            }
            (_, Some(to)) => format!(
                "  {}: {to} ({})",
                change.id,
                if change.on == Some(true) { "on" } else { "off" }
            ),
            _ => continue,
        });
    }
}

fn restored(json: bool) {
    if json {
        println!("{{\"changed\":true}}");
    } else {
        println!("Restored the previous mod selection. Restart to use it: sunrise-server restart");
    }
}

fn search(
    index: &Index,
    target: Option<&Target<'_>>,
    query: &str,
    json: bool,
) -> Result<(), String> {
    let query = query.to_lowercase();
    let results: Vec<_> = index
        .projects
        .iter()
        .filter(|project| {
            matches!(project.kind, Kind::Mod | Kind::NativeMod | Kind::Modpack)
                && [&project.id, &project.name, &project.summary]
                    .iter()
                    .any(|field| field.to_lowercase().contains(&query))
        })
        .filter_map(|project| {
            let version = if project.kind == Kind::NativeMod {
                project.versions.iter().find(|version| {
                    version.game == GAME
                        && version.capabilities.unknown().is_empty()
                        && index.revoked(&project.id, &version.file.sha256).is_none()
                        && (version.side == Some(Side::Client)
                            || target.is_some_and(|target| {
                                native::refusal(target, index, project, version).is_none()
                            }))
                })
            } else {
                index.installable(project, GAME)
            }?;
            let prebuilt = project.kind == Kind::NativeMod
                && target.is_some_and(|target| {
                    super::toolchain::host().is_ok_and(|triple| {
                        version
                            .native_binary(
                                target.build.as_str(),
                                &sunrise_mod_install::sdk::Asset::host_name(
                                    triple,
                                    sunrise_mod_install::sdk::Side::Server,
                                ),
                            )
                            .is_ok()
                    })
                });
            Some((project, version, prebuilt))
        })
        .collect();
    if json {
        let output: Vec<_> = results.iter().map(|(project, version, prebuilt)| serde_json::json!({
            "id": project.id, "version": version.version, "kind": project.kind, "side": version.side, "authors": project.authors, "summary": project.summary, "prebuilt": prebuilt,
        })).collect();
        println!("{}", serde_json::json!({"results": output}));
    } else {
        for (project, version, prebuilt) in &results {
            let side = if version.side == Some(Side::Client) {
                " (client-only: not for servers)"
            } else {
                ""
            };
            let kind = match (&project.kind, prebuilt) {
                (Kind::NativeMod, true) => "native, prebuilt",
                (Kind::NativeMod, false) => "native, source",
                (kind, _) => kind.name(),
            };
            println!(
                "{}  {}  {}  {}  {kind}{side}",
                project.id,
                version.version,
                project.authors.join(", "),
                project.summary
            );
        }
        if let Some((project, _, _)) = results
            .iter()
            .find(|(_, v, _)| v.side != Some(Side::Client))
        {
            let flag = if project.kind == Kind::Modpack {
                "--pack "
            } else {
                ""
            };
            println!("Add: sunrise-server mods add {flag}{}", project.id);
        } else if results.is_empty() {
            println!("No matching reviewed mods.");
        }
    }
    Ok(())
}
