//! No-follow opens shared by the registry serial lock and runtime leases.
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, OpenOptions};
use std::{io, path::Path};

/// Open a lock file without following its final component. The caller owns the
/// open options, lock acquisition, and release policy.
pub fn open_nofollow(
    directory: &Dir,
    path: impl AsRef<Path>,
    mut options: OpenOptions,
) -> io::Result<File> {
    directory.open_with(path, options.follow(FollowSymlinks::No))
}
