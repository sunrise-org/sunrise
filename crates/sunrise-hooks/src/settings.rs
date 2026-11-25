//! The settings store (docs/layers.md, "Platform registries"): a crate above the engine keeps its
//! own settings as `key:value` lines in the game folder, next to `options.txt`, which stays
//! exactly vanilla's. Its lines go in `sunrise-extras.txt` or `sunrise-graphics.txt` (a setting
//! that moved out of the engine's extras or video settings keeps its key there, so players keep
//! their choice) or in a file of its own.
//!
//! The engine routes the lines when it loads and saves the player's settings, never per frame.
//! With no store registered it reads and writes nothing more than before.
use std::fs;
use std::io;
use std::path::Path;

use crate::TooLate;
use crate::registry::Registry;

/// One crate's settings. The values live in the crate (statics); the engine only hands it its
/// lines and asks for them back.
pub trait Store: Sync {
    /// Its file in the game folder: `sunrise-extras.txt` or `sunrise-graphics.txt`, or its own
    /// `sunrise-<name>.txt` that the engine doesn't write.
    fn file(&self) -> &'static str;
    /// Whether a line's key is this store's.
    fn owns(&self, key: &str) -> bool;
    /// One of its lines (key and value trimmed), as the game loads the settings. A value it can't
    /// read keeps the setting as it was.
    fn load(&self, key: &str, value: &str);
    /// Its lines, as the game saves the settings.
    fn save(&self, line: &mut dyn FnMut(&str, &str));
    /// Whether every setting is at its default: then a file that doesn't exist yet isn't made.
    fn is_default(&self) -> bool;
}

static STORES: Registry<dyn Store> = Registry::new();

/// Adds `store` to those the game loads and saves. Before the game starts, as
/// [`crate::register`].
pub fn register(store: &'static dyn Store) -> Result<(), TooLate> {
    STORES.register(store)
}

pub(crate) fn all() -> &'static [&'static dyn Store] {
    STORES.all()
}

/// The engine's side: where it loads and saves the stores. Only the engine calls these.
pub mod engine {
    use super::*;

    /// Loads every store. `read(file)` is the text of a file of the game folder, if it exists;
    /// the engine passes the text it already read for a file it reads itself.
    pub fn load(read: &mut dyn FnMut(&str) -> Option<String>) {
        load_into(all(), read);
    }

    /// Appends the stores' lines of `file`, a file the engine writes itself, to `text`, its own
    /// lines. True when every store of that file is at its defaults (none counts).
    pub fn append(file: &str, text: &mut String) -> bool {
        append_lines(all(), file, text)
    }

    /// Writes the stores' own files in `dir`: every file of a store but `engine_files`, the ones
    /// the engine writes with [`append`]. A line of the file no store wrote now is kept. The files
    /// that could not be read or written come back with why; each is left as it was.
    pub fn save_own(dir: &Path, engine_files: &[&str]) -> Vec<(&'static str, io::Error)> {
        save_files(all(), dir, engine_files)
    }

    /// A settings file's text, `None` when there is none. Bytes that aren't UTF-8 read as
    /// U+FFFD, as vanilla's `Options.load` reads them, so every other line loads; such a file is
    /// `lossy`, and is never written over, so the bytes it couldn't decode stay as they are.
    pub fn read(path: &Path) -> io::Result<Option<FileText>> {
        match fs::read(path) {
            Ok(bytes) => Ok(Some(match String::from_utf8(bytes) {
                Ok(text) => FileText { text, lossy: false },
                Err(e) => FileText {
                    text: String::from_utf8_lossy(e.as_bytes()).into_owned(),
                    lossy: true,
                },
            })),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Replaces the settings file at `path` with `text`, one of two ways:
    /// - A file that doesn't exist yet, or a plain one (a regular file, not a symbolic link, with
    ///   one link, owned by this user, with no setuid, setgid or sticky bit), is replaced whole: a
    ///   temporary file in the same folder, made with the old file's mode, synced, renamed over
    ///   it, and on Unix the folder synced, so a failure or an interruption leaves the old file.
    /// - Any other regular file (reached through a link, with another hard link, another
    ///   owner's, or with a special bit) is written in place through `path`, which keeps every
    ///   attribute it has, at the cost of being cut short if the write fails partway.
    ///
    /// Anything else (a folder, a FIFO, a socket, a device) is refused and left as it is, and so
    /// is everything when its metadata can't be read for another reason than not being there.
    pub fn replace(path: &Path, text: &str) -> io::Result<()> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let found = facts(path)?;
        match way(&found) {
            Way::Refuse => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{}: not a regular file, so it is left as it is",
                    path.display()
                ),
            )),
            Way::InPlace => write_in_place(path, text),
            Way::Whole => {
                // A short name of one length, whatever the file's: a long file name can't push the
                // temporary one past the file system's limit.
                let pid = std::process::id();
                let names =
                    std::iter::repeat_with(|| NEXT.fetch_add(1, Ordering::Relaxed)).map(|n| {
                        path.with_file_name(format!(".sunrise-{pid:08x}-{:04x}.tmp", n & 0xffff))
                    });
                replace_through(path, found.old.as_ref(), text, names.take(16))
            }
        }
    }

    /// What `replace` reads of `path` before it writes: the entry itself (a link is not followed),
    /// and for a link whether it names a regular file.
    pub(super) struct Facts {
        pub(super) kind: Kind,
        /// The entry's own metadata, for a regular file that is no link.
        pub(super) old: Option<fs::Metadata>,
        /// No other hard link, no special bit, this user's: `None` where it can't be told.
        pub(super) plain: Option<bool>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Kind {
        Missing,
        File,
        /// A symbolic link to a regular file, or to nothing yet.
        LinkToFile,
        /// A folder, FIFO, socket or device, or a link to one.
        Other,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Way {
        Whole,
        InPlace,
        Refuse,
    }

    /// How `replace` writes what `facts` found.
    pub(super) fn way(facts: &Facts) -> Way {
        match (facts.kind, facts.plain) {
            (Kind::Missing, _) => Way::Whole,
            (Kind::File, Some(true)) => Way::Whole,
            (Kind::File, _) | (Kind::LinkToFile, _) => Way::InPlace,
            (Kind::Other, _) => Way::Refuse,
        }
    }

    fn facts(path: &Path) -> io::Result<Facts> {
        let entry = match fs::symlink_metadata(path) {
            Ok(entry) => entry,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Facts {
                    kind: Kind::Missing,
                    old: None,
                    plain: None,
                });
            }
            Err(e) => return Err(e),
        };
        if entry.file_type().is_symlink() {
            // The system follows the link when the file is opened; this only says what it names.
            let kind = match fs::metadata(path) {
                Ok(target) if target.is_file() => Kind::LinkToFile,
                Ok(_) => Kind::Other,
                Err(e) if e.kind() == io::ErrorKind::NotFound => Kind::LinkToFile,
                Err(e) => return Err(e),
            };
            return Ok(Facts {
                kind,
                old: None,
                plain: None,
            });
        }
        if !entry.is_file() {
            return Ok(Facts {
                kind: Kind::Other,
                old: None,
                plain: None,
            });
        }
        let plain = plain_bits(&entry);
        Ok(Facts {
            kind: Kind::File,
            old: Some(entry),
            plain,
        })
    }

    /// One link and no setuid, setgid or sticky bit (the owner is checked against the new file's,
    /// which is this process's).
    #[cfg(unix)]
    fn plain_bits(entry: &fs::Metadata) -> Option<bool> {
        use std::os::unix::fs::MetadataExt;
        Some(entry.nlink() == 1 && entry.mode() & 0o7000 == 0)
    }

    /// Outside Unix the standard library doesn't say how many links a file has, and this crate
    /// forbids the `unsafe` a system call would need to read them or the file's own access list;
    /// a regular file there is replaced whole, as before.
    #[cfg(not(unix))]
    fn plain_bits(_entry: &fs::Metadata) -> Option<bool> {
        Some(true)
    }

    /// The whole-file way, through the first of `temps` that doesn't exist yet (one that does
    /// is left alone). `old` is the file's metadata, `None` when there is no file yet.
    pub(super) fn replace_through(
        path: &Path,
        old: Option<&fs::Metadata>,
        text: &str,
        temps: impl IntoIterator<Item = std::path::PathBuf>,
    ) -> io::Result<()> {
        use std::io::Write;
        let mut taken = io::Error::new(io::ErrorKind::AlreadyExists, "no free temporary name");
        for temp in temps {
            let mut file = match open_temp(&temp, old) {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    taken = e;
                    continue;
                }
                Err(e) => return Err(e),
            };
            if let Some(old) = old
                && !same_owner(&file, old)?
            {
                // Another user's file: a new one would be this user's, so it is written in place.
                drop(file);
                let _ = fs::remove_file(&temp);
                return write_in_place(path, text);
            }
            // The old file's mode exactly: the umask may have left the new one tighter.
            let result = match old {
                Some(old) => file.set_permissions(old.permissions()),
                None => Ok(()),
            }
            .and_then(|()| file.write_all(text.as_bytes()))
            .and_then(|()| file.sync_all())
            .and_then(|()| fs::rename(&temp, path));
            if result.is_err() {
                let _ = fs::remove_file(&temp);
            }
            result?;
            return sync_folder(path);
        }
        Err(taken)
    }

    #[cfg(unix)]
    fn same_owner(file: &fs::File, old: &fs::Metadata) -> io::Result<bool> {
        use std::os::unix::fs::MetadataExt;
        Ok(file.metadata()?.uid() == old.uid())
    }

    #[cfg(not(unix))]
    fn same_owner(_file: &fs::File, _old: &fs::Metadata) -> io::Result<bool> {
        Ok(true)
    }

    /// The folder's entry for the renamed file, synced. The new file is already in place, so a
    /// failure here says so.
    #[cfg(unix)]
    fn sync_folder(path: &Path) -> io::Result<()> {
        let folder = match path.parent() {
            Some(folder) if !folder.as_os_str().is_empty() => folder,
            _ => Path::new("."),
        };
        fs::File::open(folder).and_then(|f| f.sync_all()).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "{} was replaced, but its folder didn't sync, so the change may not survive a crash: {e}",
                    path.display()
                ),
            )
        })
    }

    /// Windows has no folder to sync: `std::fs::rename` is `MoveFileExW` with
    /// `MOVEFILE_REPLACE_EXISTING`, without write-through.
    #[cfg(not(unix))]
    fn sync_folder(_path: &Path) -> io::Result<()> {
        Ok(())
    }

    /// The file at `path` itself (the system follows a link), written from its start, cut to the
    /// text's length and synced: every attribute stays as it is.
    fn write_in_place(path: &Path, text: &str) -> io::Result<()> {
        use std::io::Write;
        // Not truncated on open: the file is cut to the text's length once it is written.
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.write_all(text.as_bytes())?;
        file.set_len(text.len() as u64)?;
        file.sync_all()
    }

    /// A new temporary file, made with the old file's mode where there is one, so it is never
    /// readable by anyone the old file wasn't (the umask can only take more away).
    #[cfg(unix)]
    pub(super) fn open_temp(temp: &Path, old: Option<&fs::Metadata>) -> io::Result<fs::File> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        if let Some(old) = old {
            options.mode(old.permissions().mode() & 0o777);
        }
        options.open(temp)
    }

    /// A new temporary file: it takes its folder's permissions, as a new settings file does.
    #[cfg(not(unix))]
    pub(super) fn open_temp(temp: &Path, _old: Option<&fs::Metadata>) -> io::Result<fs::File> {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temp)
    }

    /// The keys of `text`'s `key:value` lines: what the stores wrote.
    pub fn keys(text: &str) -> Vec<&str> {
        lines(text).map(|(_, key, _)| key).collect()
    }
}

/// A settings file's text as [`engine::read`] gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileText {
    pub text: String,
    /// Some bytes weren't UTF-8 and read as U+FFFD: the file must not be written over.
    pub lossy: bool,
}

/// Each file the stores use, once, in registration order.
fn files(stores: &[&dyn Store]) -> Vec<&'static str> {
    let mut files: Vec<&'static str> = Vec::new();
    for store in stores {
        if !files.contains(&store.file()) {
            files.push(store.file());
        }
    }
    files
}

/// `key:value` lines, as the game's settings files have them.
fn lines(text: &str) -> impl Iterator<Item = (&str, &str, &str)> {
    text.lines().filter_map(|line| {
        let (key, value) = line.split_once(':')?;
        Some((line, key.trim(), value.trim()))
    })
}

fn load_into(stores: &[&dyn Store], read: &mut dyn FnMut(&str) -> Option<String>) {
    for file in files(stores) {
        let Some(text) = read(file) else {
            continue;
        };
        for (_, key, value) in lines(&text) {
            for store in stores.iter().filter(|s| s.file() == file && s.owns(key)) {
                store.load(key, value);
            }
        }
    }
}

fn append_lines(stores: &[&dyn Store], file: &str, text: &mut String) -> bool {
    let mut default = true;
    for store in stores.iter().filter(|s| s.file() == file) {
        default &= store.is_default();
        store.save(&mut |key, value| {
            text.push_str(key);
            text.push(':');
            text.push_str(value);
            text.push('\n');
        });
    }
    default
}

fn save_files(
    stores: &[&dyn Store],
    dir: &Path,
    engine_files: &[&str],
) -> Vec<(&'static str, io::Error)> {
    let mut failed = Vec::new();
    for file in files(stores) {
        if engine_files.contains(&file) {
            continue;
        }
        let path = dir.join(file);
        let existing = match engine::read(&path) {
            Ok(Some(read)) if read.lossy => {
                let why = "it has bytes that aren't UTF-8, so it is left as it is";
                failed.push((file, io::Error::new(io::ErrorKind::InvalidData, why)));
                continue;
            }
            Ok(existing) => existing.map(|read| read.text),
            Err(e) => {
                failed.push((file, e));
                continue;
            }
        };
        let mut text = String::new();
        let default = append_lines(stores, file, &mut text);
        if existing.is_none() && default {
            continue;
        }
        // Lines no store wrote now, as written: another build's, a setting whose crate isn't in
        // this one, one a store left alone this time, or one that isn't `key:value` at all.
        let written = engine::keys(&text);
        let mut kept = String::new();
        for record in existing
            .as_deref()
            .unwrap_or_default()
            .split_inclusive('\n')
        {
            let key = record.split_once(':').map(|(key, _)| key.trim());
            if !key.is_some_and(|key| written.contains(&key)) {
                kept.push_str(record);
            }
        }
        text.push_str(&kept);
        if let Err(e) = engine::replace(&path, &text) {
            failed.push((file, e));
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A store of one switch, `<prefix>.on`, off by default.
    struct Switch {
        file: &'static str,
        key: &'static str,
        on: Mutex<bool>,
    }

    impl Switch {
        const fn new(file: &'static str, key: &'static str) -> Self {
            Self {
                file,
                key,
                on: Mutex::new(false),
            }
        }
        fn on(&self) -> bool {
            *self.on.lock().unwrap()
        }
        fn set(&self, on: bool) {
            *self.on.lock().unwrap() = on;
        }
    }

    impl Store for Switch {
        fn file(&self) -> &'static str {
            self.file
        }
        fn owns(&self, key: &str) -> bool {
            key == self.key
        }
        fn load(&self, _key: &str, value: &str) {
            match value {
                "true" => self.set(true),
                "false" => self.set(false),
                _ => {}
            }
        }
        fn save(&self, line: &mut dyn FnMut(&str, &str)) {
            line(self.key, if self.on() { "true" } else { "false" });
        }
        fn is_default(&self) -> bool {
            !self.on()
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sunrise-hooks-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn each_store_reads_only_its_own_lines() {
        let zoom = Switch::new("sunrise-extras.txt", "zoom");
        let pad = Switch::new("sunrise-pad.txt", "rumble");
        let stores: [&dyn Store; 2] = [&zoom, &pad];
        let mut asked = Vec::new();
        load_into(&stores, &mut |file| {
            asked.push(file.to_owned());
            match file {
                // The engine's own lines, and a key the store's name merely starts.
                "sunrise-extras.txt" => Some("sort:true\nzoom : true\nzoomLevel:4\n".into()),
                _ => Some("zoom:true\nrumble:maybe\n".into()),
            }
        });
        assert!(zoom.on());
        // `rumble:maybe` is no value it reads, and `zoom` in its file isn't its key.
        assert!(!pad.on());
        assert_eq!(
            asked,
            ["sunrise-extras.txt", "sunrise-pad.txt"],
            "each file read once"
        );
    }

    #[test]
    fn a_shared_file_gets_the_stores_lines_after_the_engines() {
        let zoom = Switch::new("sunrise-extras.txt", "zoom");
        let pad = Switch::new("sunrise-pad.txt", "rumble");
        let stores: [&dyn Store; 2] = [&zoom, &pad];
        let mut text = "sort:true\n".to_owned();
        assert!(append_lines(&stores, "sunrise-extras.txt", &mut text));
        assert_eq!(text, "sort:true\nzoom:false\n");
        zoom.set(true);
        let mut text = String::new();
        assert!(!append_lines(&stores, "sunrise-extras.txt", &mut text));
        assert_eq!(text, "zoom:true\n");
    }

    #[test]
    fn own_files_appear_once_changed_and_keep_lines_nobody_owns() {
        let dir = scratch("own-files");
        let pad = Switch::new("sunrise-pad.txt", "rumble");
        let zoom = Switch::new("sunrise-extras.txt", "zoom");
        let stores: [&dyn Store; 2] = [&pad, &zoom];
        save_files(&stores, &dir, &["sunrise-extras.txt"]);
        assert!(
            !dir.join("sunrise-pad.txt").exists(),
            "defaults make no file"
        );
        assert!(
            !dir.join("sunrise-extras.txt").exists(),
            "the engine writes that one"
        );
        pad.set(true);
        save_files(&stores, &dir, &["sunrise-extras.txt"]);
        assert_eq!(
            fs::read_to_string(dir.join("sunrise-pad.txt")).unwrap(),
            "rumble:true\n"
        );
        // Another build's line stays; the store's own line is rewritten, not repeated.
        fs::write(dir.join("sunrise-pad.txt"), "rumble:true\ndeadzone:0.2\n").unwrap();
        pad.set(false);
        save_files(&stores, &dir, &["sunrise-extras.txt"]);
        assert_eq!(
            fs::read_to_string(dir.join("sunrise-pad.txt")).unwrap(),
            "rumble:false\ndeadzone:0.2\n"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// Owns `presence` but writes it only on the first seat, as a per-seat store does.
    struct FirstSeatOnly;
    impl Store for FirstSeatOnly {
        fn file(&self) -> &'static str {
            "sunrise-pad.txt"
        }
        fn owns(&self, key: &str) -> bool {
            key == "presence"
        }
        fn load(&self, _: &str, _: &str) {}
        fn save(&self, _: &mut dyn FnMut(&str, &str)) {}
        fn is_default(&self) -> bool {
            false
        }
    }

    #[test]
    fn a_line_its_store_did_not_write_this_time_is_kept() {
        let dir = scratch("unwritten");
        fs::write(dir.join("sunrise-pad.txt"), "presence:true\n").unwrap();
        let stores: [&dyn Store; 1] = [&FirstSeatOnly];
        assert!(save_files(&stores, &dir, &[]).is_empty());
        assert_eq!(
            fs::read_to_string(dir.join("sunrise-pad.txt")).unwrap(),
            "presence:true\n"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_replace_leaves_the_old_file_and_no_temporary() {
        let dir = scratch("replace");
        let target = dir.join("sunrise-pad.txt");
        // A directory in the file's place is no regular file: refused, before anything is written.
        fs::create_dir_all(target.join("kept")).unwrap();
        assert!(engine::replace(&target, "rumble:true\n").is_err());
        assert!(target.join("kept").is_dir());
        let left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, [std::ffi::OsString::from("sunrise-pad.txt")]);
        let _ = fs::remove_dir_all(dir);
    }

    /// The names in `dir`, sorted: what a replace left there.
    #[cfg(unix)]
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[cfg(unix)]
    #[test]
    fn replacing_a_linked_file_replaces_what_it_names_and_keeps_the_link() {
        use std::os::unix::fs::symlink;
        let dir = scratch("replace-link");
        let real = dir.join("real");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("sunrise-pad.txt"), "rumble:false\n").unwrap();
        let link = dir.join("sunrise-pad.txt");
        symlink("real/sunrise-pad.txt", &link).unwrap();
        engine::replace(&link, "rumble:true\n").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(&link).unwrap(),
            Path::new("real/sunrise-pad.txt")
        );
        assert_eq!(
            fs::read_to_string(real.join("sunrise-pad.txt")).unwrap(),
            "rumble:true\n"
        );
        // Written in place: no temporary file in either folder.
        assert_eq!(names(&dir), ["real", "sunrise-pad.txt"]);
        assert_eq!(names(&real), ["sunrise-pad.txt"]);
        // A link to that link ends at the same file; one to a file not made yet makes it.
        symlink("sunrise-pad.txt", dir.join("again.txt")).unwrap();
        engine::replace(&dir.join("again.txt"), "rumble:false\n").unwrap();
        assert_eq!(
            fs::read_to_string(real.join("sunrise-pad.txt")).unwrap(),
            "rumble:false\n"
        );
        symlink("real/new.txt", dir.join("new.txt")).unwrap();
        engine::replace(&dir.join("new.txt"), "zoom:true\n").unwrap();
        assert!(
            fs::symlink_metadata(dir.join("new.txt"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(real.join("new.txt")).unwrap(),
            "zoom:true\n"
        );
        // A loop of links is an error, and nothing is written.
        symlink("loop-b.txt", dir.join("loop-a.txt")).unwrap();
        symlink("loop-a.txt", dir.join("loop-b.txt")).unwrap();
        assert!(engine::replace(&dir.join("loop-a.txt"), "x:1\n").is_err());
        assert!(
            fs::symlink_metadata(dir.join("loop-a.txt"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// The temporary file is made with the old file's mode, never looser, before a byte of the
    /// text is in it.
    #[cfg(unix)]
    #[test]
    fn the_temporary_file_is_never_looser_than_the_one_it_replaces() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("replace-temp-mode");
        let path = dir.join("sunrise-extras.txt");
        fs::write(&path, "zoom:false\n").unwrap();
        for mode in [0o600, 0o640, 0o400] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            let temp = dir.join(format!(".temp-{mode:o}"));
            let old = fs::metadata(&path).unwrap();
            drop(engine::open_temp(&temp, Some(&old)).unwrap());
            let made = fs::metadata(&temp).unwrap().permissions().mode() & 0o777;
            assert_eq!(made & !mode, 0, "{made:o} is looser than {mode:o}");
            fs::remove_file(&temp).unwrap();
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn only_a_missing_or_plain_file_is_replaced_whole() {
        use engine::{Facts, Kind, Way, way};
        let facts = |kind, plain| Facts {
            kind,
            old: None,
            plain,
        };
        assert_eq!(way(&facts(Kind::Missing, None)), Way::Whole);
        assert_eq!(way(&facts(Kind::File, Some(true))), Way::Whole);
        // Another hard link, a special bit, or anything that can't be told: in place.
        assert_eq!(way(&facts(Kind::File, Some(false))), Way::InPlace);
        assert_eq!(way(&facts(Kind::File, None)), Way::InPlace);
        // Through a link the system follows it, whatever the link names.
        assert_eq!(way(&facts(Kind::LinkToFile, Some(true))), Way::InPlace);
        assert_eq!(way(&facts(Kind::LinkToFile, None)), Way::InPlace);
        // Not a regular file, or a link to one that isn't: never written.
        assert_eq!(way(&facts(Kind::Other, None)), Way::Refuse);
    }

    #[cfg(unix)]
    #[test]
    fn a_plain_file_is_replaced_whole_and_keeps_its_mode() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = scratch("replace-plain");
        // A fresh file for each mode: 0400 can't be written again by the test itself.
        for mode in [0o600, 0o640, 0o644, 0o400] {
            let path = dir.join(format!("sunrise-extras-{mode:o}.txt"));
            fs::write(&path, "zoom:false\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            let before = fs::metadata(&path).unwrap();
            engine::replace(&path, "zoom:true\n").unwrap();
            let after = fs::metadata(&path).unwrap();
            assert_ne!(
                after.ino(),
                before.ino(),
                "{mode:o}: a new file, renamed over"
            );
            assert_eq!(after.mode() & 0o7777, mode);
            assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
            assert_eq!(fs::read_to_string(&path).unwrap(), "zoom:true\n");
        }
        let mut left = names(&dir);
        left.retain(|n| n.ends_with(".tmp"));
        assert!(left.is_empty(), "{left:?}");
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn what_isnt_a_regular_file_is_refused_and_left_as_it_is() {
        use std::os::unix::fs::{FileTypeExt, symlink};
        use std::os::unix::net::UnixListener;
        let dir = scratch("replace-refused");
        // A socket stands in for a FIFO or a device: none of them is a regular file.
        let socket = dir.join("sunrise-pad.txt");
        let _listener = UnixListener::bind(&socket).unwrap();
        symlink("sunrise-pad.txt", dir.join("linked.txt")).unwrap();
        for path in [socket.clone(), dir.join("linked.txt")] {
            let error = engine::replace(&path, "rumble:true\n").unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
        }
        assert!(
            fs::symlink_metadata(&socket)
                .unwrap()
                .file_type()
                .is_socket()
        );
        assert!(
            fs::symlink_metadata(dir.join("linked.txt"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(names(&dir), ["linked.txt", "sunrise-pad.txt"]);
        let _ = fs::remove_dir_all(dir);
    }

    /// A file name near the system's limit: the temporary name has one short length whatever the
    /// file's, and through a link the file is written in place.
    #[cfg(unix)]
    #[test]
    fn a_long_file_name_is_replaced_plain_and_through_a_link() {
        use std::os::unix::fs::symlink;
        let dir = scratch("replace-long-name");
        let long = format!("{}.txt", "s".repeat(240));
        fs::write(dir.join(&long), "zoom:false\n").unwrap();
        engine::replace(&dir.join(&long), "zoom:true\n").unwrap();
        assert_eq!(fs::read_to_string(dir.join(&long)).unwrap(), "zoom:true\n");
        let real = dir.join("real");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join(&long), "zoom:false\n").unwrap();
        symlink(format!("real/{long}"), dir.join("link.txt")).unwrap();
        engine::replace(&dir.join("link.txt"), "zoom:true\n").unwrap();
        assert_eq!(fs::read_to_string(real.join(&long)).unwrap(), "zoom:true\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_hard_linked_file_is_written_in_place_and_keeps_its_links() {
        use std::os::unix::fs::MetadataExt;
        let dir = scratch("replace-hard-link");
        let path = dir.join("sunrise-pad.txt");
        let other = dir.join("backup.txt");
        fs::write(&path, "rumble:false\n").unwrap();
        fs::hard_link(&path, &other).unwrap();
        let before = fs::metadata(&path).unwrap();
        engine::replace(&path, "rumble:true\n").unwrap();
        let after = fs::metadata(&path).unwrap();
        assert_eq!((after.ino(), after.nlink()), (before.ino(), 2));
        assert_eq!(fs::read_to_string(&other).unwrap(), "rumble:true\n");
        assert_eq!(names(&dir), ["backup.txt", "sunrise-pad.txt"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_file_is_written_in_place_and_keeps_its_owner_group_and_mode() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
        let dir = scratch("replace-owner");
        let real = dir.join("real");
        fs::create_dir_all(&real).unwrap();
        let file = real.join("sunrise-extras.txt");
        fs::write(&file, "zoom:false\n").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
        symlink("real/sunrise-extras.txt", dir.join("sunrise-extras.txt")).unwrap();
        let before = fs::metadata(&file).unwrap();
        engine::replace(&dir.join("sunrise-extras.txt"), "zoom:true\n").unwrap();
        let after = fs::metadata(&file).unwrap();
        assert_eq!(
            (
                after.ino(),
                after.uid(),
                after.gid(),
                after.mode() & 0o7777,
                after.nlink()
            ),
            (before.ino(), before.uid(), before.gid(), 0o640, 1)
        );
        let link = fs::symlink_metadata(dir.join("sunrise-extras.txt")).unwrap();
        assert!(link.file_type().is_symlink());
        assert_eq!(fs::read_to_string(&file).unwrap(), "zoom:true\n");
        let _ = fs::remove_dir_all(dir);
    }

    /// A file whose metadata can't be read (here, behind a folder this user may not search) is
    /// not taken for a new one: the save stops, and the file stays as it was.
    #[cfg(unix)]
    #[test]
    fn a_metadata_error_stops_the_save_and_leaves_the_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("replace-metadata-error");
        let locked = dir.join("locked");
        fs::create_dir_all(&locked).unwrap();
        let path = locked.join("sunrise-pad.txt");
        fs::write(&path, "rumble:false\n").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o600)).unwrap();
        // Where permissions aren't enforced (root), the metadata reads and nothing fails.
        let enforced = fs::metadata(&path).is_err();
        let result = engine::replace(&path, "rumble:true\n");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if enforced {
            let error = result.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
            assert_eq!(fs::read_to_string(&path).unwrap(), "rumble:false\n");
            assert_eq!(names(&locked), ["sunrise-pad.txt"]);
        }
        let _ = fs::remove_dir_all(dir);
    }

    /// A store's own file, saved by the engine: through a link, and private, it stays both.
    #[cfg(unix)]
    #[test]
    fn a_stores_own_file_keeps_its_link_and_mode() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = scratch("store-link-mode");
        let real = dir.join("synced");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("sunrise-pad.txt"), "rumble:false\n").unwrap();
        fs::set_permissions(
            real.join("sunrise-pad.txt"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        symlink("synced/sunrise-pad.txt", dir.join("sunrise-pad.txt")).unwrap();
        let pad = Switch::new("sunrise-pad.txt", "rumble");
        pad.set(true);
        let stores: [&dyn Store; 1] = [&pad];
        save_files(&stores, &dir, &["sunrise-extras.txt"]);
        assert!(
            fs::symlink_metadata(dir.join("sunrise-pad.txt"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let saved = real.join("sunrise-pad.txt");
        assert_eq!(fs::read_to_string(&saved).unwrap(), "rumble:true\n");
        assert_eq!(
            fs::metadata(&saved).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_with_a_bad_byte_loads_its_other_lines_and_is_never_written_over() {
        let dir = scratch("bad-byte");
        let path = dir.join("sunrise-pad.txt");
        let bytes = b"rumble:false\nname:\xff\nno colon \xfe\r\n";
        fs::write(&path, bytes).unwrap();
        let read = engine::read(&path).unwrap().unwrap();
        assert!(read.lossy && read.text.starts_with("rumble:false\n"));
        assert_eq!(engine::read(&dir.join("absent.txt")).unwrap(), None);
        let pad = Switch::new("sunrise-pad.txt", "rumble");
        pad.set(true);
        let stores: [&dyn Store; 1] = [&pad];
        assert_eq!(save_files(&stores, &dir, &[]).len(), 1, "the save says why");
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn kept_records_are_saved_as_written_the_last_one_unterminated() {
        let dir = scratch("kept-records");
        let path = dir.join("sunrise-pad.txt");
        fs::write(&path, "rumble:false\r\nno colon here\r\nother : 2 ").unwrap();
        let pad = Switch::new("sunrise-pad.txt", "rumble");
        pad.set(true);
        let stores: [&dyn Store; 1] = [&pad];
        assert!(save_files(&stores, &dir, &[]).is_empty());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "rumble:true\nno colon here\r\nother : 2 "
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_at_the_temporary_name_is_left_alone() {
        let dir = scratch("temp-taken");
        let target = dir.join("sunrise-pad.txt");
        let (taken, free) = (dir.join(".taken.tmp"), dir.join(".free.tmp"));
        fs::write(&taken, "someone else's").unwrap();
        engine::replace_through(
            &target,
            None,
            "rumble:true\n",
            [taken.clone(), free.clone()],
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&taken).unwrap(), "someone else's");
        assert_eq!(fs::read_to_string(&target).unwrap(), "rumble:true\n");
        assert!(!free.exists());
        let _ = fs::remove_dir_all(dir);
    }
}
