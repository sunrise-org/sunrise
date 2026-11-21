//! A package's runtime data files, read through its registration context.
//!
//! A mod's package may carry a `runtime/` folder of data its code opens beside itself (a
//! compiled script bundle, a baked lookup table), each file listed with its SHA-512 in the
//! manifest's `[runtime]` table. The game checks every hash once, when the package opens, and
//! serves those bytes: a read never hashes again. A mod reads only its own package's files.
use std::sync::Arc;

/// The game's side of one package's runtime files. A mod reaches it through
/// [`RegistrationContext::runtime_files`](crate::RegistrationContext::runtime_files), not
/// directly.
pub trait RuntimeFileSource: Send + Sync + 'static {
    /// The names of the files the manifest lists, in order.
    fn names(&self) -> Vec<String>;
    /// The checked bytes of `name`; `None` for a name the manifest does not list.
    fn file(&self, name: &str) -> Option<Vec<u8>>;
    /// Where `name` sits on disk, when the package is a folder and lists it, as the host's own
    /// path (never through UTF-8 text).
    fn path(&self, name: &str) -> Option<crate::InstanceDirectory>;
}

/// A package's runtime files were asked for and are not there.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum RuntimeFilesError {
    /// The calling mod's package declares no `[runtime]`, or the call is outside any mod's
    /// dispatch (a test, a tool).
    NoRuntimeFiles,
}
impl std::fmt::Display for RuntimeFilesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRuntimeFiles => f.write_str("this mod's package declares no [runtime] files"),
        }
    }
}
impl std::error::Error for RuntimeFilesError {}

/// A path read of a runtime file was refused.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum RuntimePathError {
    /// The package does not list this name.
    NotListed,
    /// The package is a `.smod` archive, which has no file to point at; read the bytes.
    NotAFolder,
}
impl std::fmt::Display for RuntimePathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotListed => "the package's [runtime] does not list this file",
            Self::NotAFolder => {
                "the package is a .smod archive, so its runtime files have no path: read the bytes"
            }
        })
    }
}
impl std::error::Error for RuntimePathError {}

/// The calling mod's own runtime files, from
/// [`RegistrationContext::runtime_files`](crate::RegistrationContext::runtime_files). The mod may
/// keep it and read from any thread: the bytes were checked against the manifest's SHA-512 when the
/// package opened, and no read checks them again.
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: a mod-side handle around the game's source, which crosses as its own resource.
#[derive(Clone)]
pub struct RuntimeFiles {
    source: Arc<dyn RuntimeFileSource>,
}

impl RuntimeFiles {
    /// The names of the files the package lists, in order.
    pub fn names(&self) -> Vec<String> {
        self.source.names()
    }

    /// The bytes of `name`, a name the manifest lists under `[runtime]` (not `runtime/` before
    /// it). `None` for any other name: a mod cannot read a file its package does not list, nor
    /// another package's.
    pub fn file(&self, name: &str) -> Option<Vec<u8>> {
        self.source.file(name)
    }

    /// Where `name` sits on disk, so native code can map or cache-compile a large file without a
    /// copy. A package that is a `.smod` archive has no path ([`RuntimePathError::NotAFolder`]).
    /// The file was checked when the package opened; a later edit of the folder is the mod's to
    /// guard against.
    pub fn file_path(&self, name: &str) -> Result<std::path::PathBuf, RuntimePathError> {
        if !self.source.names().iter().any(|listed| listed == name) {
            return Err(RuntimePathError::NotListed);
        }
        self.source
            .path(name)
            .and_then(|path| path.to_path_buf())
            .ok_or(RuntimePathError::NotAFolder)
    }
}

impl crate::RegistrationContext {
    /// The runtime files of the mod this call runs for, from its package's `[runtime]`. Available
    /// from registration on, before any level exists; keep the handle to read later.
    /// `NoRuntimeFiles` when the package declares none.
    pub fn runtime_files(&self) -> Result<RuntimeFiles, RuntimeFilesError> {
        self.bound_runtime_files()
            .map(|source| RuntimeFiles { source })
            .ok_or(RuntimeFilesError::NoRuntimeFiles)
    }

    /// The source for the mod being dispatched.
    pub(crate) fn bound_runtime_files(&self) -> Option<Arc<dyn RuntimeFileSource>> {
        let owner = crate::content::crash::current_native_mod()?;
        self.data.as_ref()?.runtime_source(owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One package's files: `nav.bin` is listed and, when `folder`, sits on disk.
    struct Fake {
        folder: bool,
    }
    impl RuntimeFileSource for Fake {
        fn names(&self) -> Vec<String> {
            vec!["nav.bin".into()]
        }
        fn file(&self, name: &str) -> Option<Vec<u8>> {
            (name == "nav.bin").then(|| b"tables".to_vec())
        }
        fn path(&self, name: &str) -> Option<crate::InstanceDirectory> {
            (self.folder && name == "nav.bin").then(|| host_path().0)
        }
    }

    /// A host path with a byte (or code unit) no UTF-8 text holds, as the value the game hands
    /// over and as the path it must give back.
    #[cfg(unix)]
    fn host_path() -> (crate::InstanceDirectory, std::path::PathBuf) {
        use std::os::unix::ffi::OsStrExt;
        let bytes = b"/mods/\xff ruby/runtime/nav.bin";
        (
            crate::InstanceDirectory::Unix(bytes.to_vec()),
            std::path::PathBuf::from(std::ffi::OsStr::from_bytes(bytes)),
        )
    }
    #[cfg(windows)]
    fn host_path() -> (crate::InstanceDirectory, std::path::PathBuf) {
        use std::os::windows::ffi::OsStringExt;
        let mut units: Vec<u16> = "C:\\mods\\".encode_utf16().collect();
        units.push(0xd800);
        units.extend("ruby\\runtime\\nav.bin".encode_utf16());
        (
            crate::InstanceDirectory::Windows(units.clone()),
            std::path::PathBuf::from(std::ffi::OsString::from_wide(&units)),
        )
    }
    #[cfg(not(any(unix, windows)))]
    fn host_path() -> (crate::InstanceDirectory, std::path::PathBuf) {
        (
            crate::InstanceDirectory::Unix(b"/mods/ruby/runtime/nav.bin".to_vec()),
            std::path::PathBuf::new(),
        )
    }

    fn files(folder: bool) -> RuntimeFiles {
        RuntimeFiles {
            source: Arc::new(Fake { folder }),
        }
    }

    #[test]
    fn a_mod_reads_only_the_names_its_manifest_lists() {
        let files = files(true);
        assert_eq!(files.names(), ["nav.bin"]);
        assert_eq!(files.file("nav.bin").as_deref(), Some(b"tables".as_slice()));
        assert_eq!(files.file("other.bin"), None);
        assert_eq!(files.file("../nav.bin"), None);
    }

    #[test]
    fn a_mod_without_a_runtime_table_is_told_so() {
        assert_eq!(
            crate::RegistrationContext::new().runtime_files().err(),
            Some(RuntimeFilesError::NoRuntimeFiles)
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn a_path_is_listed_only_and_a_folder_only_and_comes_back_exactly() {
        let folder = files(true);
        let archive = files(false);
        assert_eq!(folder.file_path("nav.bin"), Ok(host_path().1));
        assert_eq!(
            folder.file_path("other.bin"),
            Err(RuntimePathError::NotListed)
        );
        assert_eq!(
            archive.file_path("nav.bin"),
            Err(RuntimePathError::NotAFolder)
        );
    }
}
