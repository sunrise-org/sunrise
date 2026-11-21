//! One write-ahead swap covers the server's mod settings, packages and runtime. The journal
//! is private to the installer; startup refuses it instead of reading a half-applied set.
use crate::{
    input::Mods,
    lock::{self, ModLock},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};
use sunrise_mod_package::{durable, mod_list};
use toml_edit::{DocumentMut, Item};

/// The snapshot that rollback restores.
pub const PREVIOUS_DIR: &str = ".mods-previous";
/// The mod set saved when changing the installed Sunrise release.
pub const RELEASE_PREVIOUS: &str = ".mods-release-previous";
pub const RELEASE_PENDING: &str = ".mods-release-pending";
const JOURNAL: &str = ".mods-transaction";
const COMPLETE: &str = ".mods-transaction-complete";
const GUARD: &str = ".mods-write.lock";
const CONFIG: &str = "server.toml";
pub const PACK_FILE: &str = ".mods-pack.spack";
pub const NATIVE_DIR: &str = "native";
const FILES: &[&str] = &[
    CONFIG,
    lock::LOCK,
    mod_list::FILE,
    mod_list::DIR,
    PACK_FILE,
    NATIVE_DIR,
];

pub struct ServerFolder {
    root: PathBuf,
    _guard: File,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Move {
    old: [bool; FILES.len()],
    new: [bool; FILES.len()],
    undo: Undo,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Undo {
    KeepMods,
    ConsumeMods,
    KeepRelease,
    ConsumeRelease,
}

pub struct Stage(tempfile::TempDir);
impl Stage {
    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

/// A read-only lease keeps the final swap from crossing a startup read. An unmanaged folder
/// need not have a lock file; in particular a failed preflight does not create one.
pub fn read_lease(root: &Path) -> Result<Option<File>, String> {
    let lease = match File::open(root.join(GUARD)) {
        Ok(file) => {
            file.try_lock_shared()
                .map_err(|e| format!("the mod selection is in use: {e}"))?;
            Some(file)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{GUARD}: {e}")),
    };
    if root.join(JOURNAL).try_exists().map_err(|e| e.to_string())? {
        return Err(
            "the last mod change was interrupted\n  Recover it: sunrise-server mods rollback"
                .into(),
        );
    }
    Ok(lease)
}

impl ServerFolder {
    /// Serializes writers for the entire read/plan/stage/swap operation. No existing files are
    /// touched here; an interrupted transaction is recovered explicitly, not while starting.
    pub fn open(root: &Path) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|e| format!("{}: {e}", root.display()))?;
        let guard = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(GUARD))
            .map_err(|e| format!("{GUARD}: {e}"))?;
        guard
            .try_lock()
            .map_err(|e| format!("the mod selection is in use: {e}"))?;
        Ok(Self {
            root,
            _guard: guard,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn stage(&self) -> Result<Stage, String> {
        if self
            .root
            .join(JOURNAL)
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            return Err(
                "the last mod change was interrupted\n  Recover it: sunrise-server mods rollback"
                    .into(),
            );
        }
        self.scratch()
    }

    fn scratch(&self) -> Result<Stage, String> {
        tempfile::Builder::new()
            .prefix(".mods-stage-")
            .tempdir_in(&self.root)
            .map(Stage)
            .map_err(|e| format!("cannot stage the mod selection: {e}"))
    }

    /// Apply the staged files. `config` is exactly the configuration read before planning;
    /// preserve edits that occur while downloading by refusing a stale plan rather than overwriting.
    pub fn commit(&self, stage: Stage, config: &str) -> Result<(), String> {
        self.reread(config)?;
        self.swap(stage, Undo::KeepMods)
    }

    /// Replace the release's mod set but keep a separate snapshot until its engine starts.
    pub fn commit_release(&self, stage: Stage, config: &str) -> Result<(), String> {
        self.reread(config)?;
        if self
            .root
            .join(RELEASE_PENDING)
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            return Err(
                "a release mod change is still pending; recover it before updating again".into(),
            );
        }
        self.swap(stage, Undo::KeepRelease)
    }

    pub fn finish_release(&self, switched_tag: bool) -> Result<(), String> {
        let pending = self.root.join(RELEASE_PENDING);
        let result = if switched_tag {
            let previous = self.root.join(RELEASE_PREVIOUS);
            remove(&previous).and_then(|()| rename(&pending, &previous))
        } else {
            remove(&pending)
        };
        result.map_err(|e| format!("couldn't finish the mod release change: {e}"))
    }

    /// Restore the mods of a release that never booted; do not touch ordinary mods undo.
    pub fn undo_release(&self, config: &str) -> Result<(), String> {
        let stage = self.stage_previous(RELEASE_PENDING, config)?;
        self.reread(config)?;
        self.swap(stage, Undo::ConsumeRelease)
    }

    pub fn previous_lock(&self) -> Result<Option<ModLock>, String> {
        let previous = self.root.join(PREVIOUS_DIR);
        if previous
            .join(lock::LOCK)
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            ModLock::read(&previous).map(Some)
        } else if previous.is_dir() {
            Ok(None)
        } else {
            Err("there is no previous mod selection".into())
        }
    }

    /// Replay an unfinished swap. Return true if there was a swap to recover, so the command
    /// doesn't then undo the previous, unrelated change as well.
    pub fn recover(&self) -> Result<bool, String> {
        let folder = self.root.join(JOURNAL);
        if !folder.try_exists().map_err(|e| e.to_string())? {
            return Ok(false);
        }
        finish(&self.root, &folder)
            .map_err(|e| format!("cannot recover the mod selection: {e}"))?;
        Ok(true)
    }

    /// Restore the previous selection, but keep unrelated server.toml edits made since the swap.
    /// Consume the previous selection after success; this is undo, not a toggle.
    pub fn rollback(&self, config: &str) -> Result<(), String> {
        let stage = self.stage_previous(PREVIOUS_DIR, config)?;
        self.reread(config)?;
        self.swap(stage, Undo::ConsumeMods)
    }

    pub fn stage_release_previous(&self, config: &str) -> Result<Stage, String> {
        self.stage_previous(RELEASE_PREVIOUS, config)
    }

    fn stage_previous(&self, snapshot: &str, config: &str) -> Result<Stage, String> {
        let previous = self.root.join(snapshot);
        if !previous.is_dir() {
            return Err("there is no previous mod selection".into());
        }
        let stage = self.stage()?;
        for name in FILES.iter().copied().filter(|name| *name != CONFIG) {
            copy_tree(&previous.join(name), &stage.path().join(name))
                .map_err(|e| format!("{name}: {e}"))?;
        }
        // A package rollback must not roll back data written by the game after installation.
        let storage = Path::new(mod_list::DIR).join(sunrise_mod_package::manifest::STORAGE_DIR);
        remove(&stage.path().join(&storage)).map_err(|e| format!("mod storage: {e}"))?;
        if self
            .root
            .join(&storage)
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            fs::create_dir_all(stage.path().join(mod_list::DIR)).map_err(|e| e.to_string())?;
            copy_tree(&self.root.join(&storage), &stage.path().join(&storage))
                .map_err(|e| format!("mod storage: {e}"))?;
        }
        let prior = match fs::read_to_string(previous.join(CONFIG)) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{snapshot}/{CONFIG}: {e}")),
        };
        let mut new = config.parse::<DocumentMut>().map_err(|e| e.to_string())?;
        let prior = prior.parse::<DocumentMut>().map_err(|e| e.to_string())?;
        match prior.get("mods") {
            Some(mods) => {
                new["mods"] = mods.clone();
            }
            None => {
                new.remove("mods");
            }
        }
        write_record(&self.root, stage.path(), CONFIG, &new.to_string())?;
        Ok(stage)
    }

    fn reread(&self, expected: &str) -> Result<(), String> {
        let path = self.root.join(CONFIG);
        let actual = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound && expected.is_empty() => String::new(),
            Err(e) => return Err(format!("{CONFIG}: {e}")),
        };
        if actual != expected {
            return Err(format!("{CONFIG} changed while preparing mods"));
        }
        Ok(())
    }

    fn swap(&self, stage: Stage, undo: Undo) -> Result<(), String> {
        let root = &self.root;
        let target = root.join(JOURNAL);
        let prepare = (|| -> io::Result<Move> {
            let mut record = Move {
                old: [false; FILES.len()],
                new: [false; FILES.len()],
                undo,
            };
            for (i, name) in FILES.iter().enumerate() {
                record.old[i] = replaceable(&root.join(name))?;
                record.new[i] = replaceable(&stage.path().join(name))?;
            }
            Ok(record)
        })();
        let record = prepare.map_err(|e| format!("cannot stage mods: {e}"))?;
        fs::create_dir(&target).map_err(|e| format!("{JOURNAL}: {e}"))?;
        let begin = (|| -> io::Result<()> {
            // The stage stays disposable until the journal owns it.
            fs::create_dir(target.join("old"))?;
            rename(stage.path(), &target.join("new"))?;
            durable::write(&target.join("move.json"), &serde_json::to_vec(&record)?)?;
            for (i, name) in FILES.iter().enumerate() {
                if record.old[i] {
                    rename(&root.join(name), &target.join("old").join(name))?;
                }
                if record.new[i] {
                    rename(&target.join("new").join(name), &root.join(name))?;
                }
            }
            durable::write(&target.join("committed"), b"")
        })();
        let result = finish(root, &target);
        match (begin, result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(e), Ok(())) => Err(format!("mod swap did not finish: {e}")),
            (started, Err(e)) => Err(format!(
                "mod swap needs recovery: {}{e}\n  Recover it: sunrise-server mods rollback",
                started
                    .err()
                    .map_or_else(String::new, |cause| format!("{cause}; "))
            )),
        }
    }
}

/// Change only `[mods]`; all other settings and comments survive.
pub fn config_with_mods(text: &str, mods: &Mods) -> Result<String, String> {
    let mut doc = text.parse::<DocumentMut>().map_err(|e| e.to_string())?;
    let serialized = toml::to_string(mods).map_err(|e| e.to_string())?;
    let table = serialized
        .parse::<DocumentMut>()
        .map_err(|e| e.to_string())?;
    doc["mods"] = Item::Table(table.into_table());
    Ok(doc.to_string())
}

pub(crate) fn write_record(
    root: &Path,
    stage: &Path,
    name: &str,
    text: &str,
) -> Result<(), String> {
    let path = stage.join(name);
    let write = || -> io::Result<()> {
        durable::write(&path, text.as_bytes())?;
        match fs::metadata(root.join(name)) {
            Ok(meta) => fs::set_permissions(&path, meta.permissions())?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        File::open(path)?.sync_all()
    };
    write().map_err(|e| format!("{name}: {e}"))
}

fn finish(root: &Path, folder: &Path) -> io::Result<()> {
    let old = folder.join("old");
    if folder.join("committed").try_exists()? {
        let record = read_move(folder)?;
        let (snapshot, keep) = match record.undo {
            Undo::KeepMods => (PREVIOUS_DIR, true),
            Undo::ConsumeMods => (PREVIOUS_DIR, false),
            Undo::KeepRelease => (RELEASE_PENDING, true),
            Undo::ConsumeRelease => (RELEASE_PENDING, false),
        };
        let previous = root.join(snapshot);
        let discard = folder.join("discard");
        if keep && old.try_exists()? {
            if previous.try_exists()? {
                rename(&previous, &discard)?;
            }
            rename(&old, &previous)?;
        } else if !keep && previous.try_exists()? {
            rename(&previous, &discard)?;
        }
    } else if folder.join("move.json").try_exists()? {
        let record = read_move(folder)?;
        for (i, name) in FILES.iter().enumerate().rev() {
            let live = root.join(name);
            let saved = old.join(name);
            if saved.try_exists()? {
                remove(&live)?;
                rename(&saved, &live)?;
            } else if !record.old[i]
                && record.new[i]
                && !folder.join("new").join(name).try_exists()?
            {
                remove(&live)?;
            }
        }
    }
    // Detach the completed journal before deleting it; cleanup is not a recovery decision.
    let complete = root.join(COMPLETE);
    remove(&complete)?;
    rename(folder, &complete)?;
    remove(&complete)
}

fn read_move(folder: &Path) -> io::Result<Move> {
    let path = folder.join("move.json");
    serde_json::from_str(&lock::read_text(&path, 4096).map_err(io::Error::other)?)
        .map_err(Into::into)
}

fn replaceable(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() || meta.is_dir() => Ok(true),
        Ok(_) => Err(io::Error::other(format!(
            "{} is not a regular file or directory",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn rename(from: &Path, to: &Path) -> io::Result<()> {
    durable::replace(from, to)?;
    durable::sync_parent(from)
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    }
    durable::sync_parent(path)
}

pub(crate) fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(from) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.is_file() {
        fs::copy(from, to)?;
        File::open(to)?.sync_all()?;
    } else if meta.is_dir() {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        durable::sync_parent(&to.join(".sync"))?;
    } else {
        return Err(io::Error::other(format!(
            "{} is not a regular file or directory",
            from.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
