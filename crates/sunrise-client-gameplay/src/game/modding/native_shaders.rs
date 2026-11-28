//! The pick in `shaderpacks/` as the game tracks it (`game/interface/shader_packs.rs`), and what
//! became of each pack the loader was given. Sunrise's own WGSL packs (a folder with
//! `sunrise-shaders.toml`, or a `.sshader`) are no longer run: the pick of one falls back to no
//! pack with a note (`shader_pack_list::UNSUPPORTED_NOTE`).

use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

use sunrise_client_render::graphics::IrisCompat;

/// What the loader was given of a pack and said of it: the pack's files when it was read
/// (`fingerprint`), its Iris compatibility switch, and whether it was accepted.
type Outcome = (Option<u64>, IrisCompat, bool);

thread_local! {
    // Loader outcomes survive renderer replacement; rejected files must not cause a rebuild loop.
    static OUTCOMES: std::cell::RefCell<std::collections::HashMap<PathBuf, Outcome>> =
        std::cell::RefCell::default();
    // The device (`Renderer::device_id`) of the renderer installed last; `None` before one was.
    static DEVICE: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// A renderer on the device `device` (`Renderer::device_id`) is installed
/// (`ClientGame::set_renderer`). On another device than the last renderer's (a device recovery,
/// any newly opened device) every refusal is forgotten: the same files on another device are
/// other build inputs (its feature answers, such as `IRIS_FEATURE_PER_BUFFER_BLENDING`, its
/// extensions and its limits), tried again once. Acceptances are kept, and a renderer made again
/// on the same device keeps everything, so a refusal never makes a rebuild loop.
pub(in crate::game) fn follow_device(device: u64) {
    if DEVICE.replace(Some(device)) != Some(device) {
        OUTCOMES.with_borrow_mut(|packs| packs.retain(|_, (_, _, accepted)| *accepted));
    }
}

/// Whether the loader accepted the pack at `path` with the Iris compatibility switch `mode`;
/// `None` before it answered for that switch.
pub(in crate::game) fn outcome(path: &Path, mode: IrisCompat) -> Option<bool> {
    OUTCOMES.with_borrow(|packs| {
        let (_, was, accepted) = packs.get(path)?;
        (*was == mode).then_some(*accepted)
    })
}

pub(in crate::game) fn remember(path: &Path, stamp: Option<u64>, mode: IrisCompat, accepted: bool) {
    OUTCOMES.with_borrow_mut(|packs| packs.insert(path.to_owned(), (stamp, mode, accepted)));
}

/// Whether the loader refused the pack at `path` as it is now (`stamp`), with the switch `mode`:
/// the same files with another switch are other build inputs, tried again.
pub(in crate::game) fn refused(path: &Path, stamp: Option<u64>, mode: IrisCompat) -> bool {
    OUTCOMES.with_borrow(|packs| packs.get(path) == Some(&(stamp, mode, false)))
}

#[derive(Default)]
pub(in crate::game) struct NativeShaders {
    /// The path of the picked pack (`game/interface/shader_packs.rs`); empty: none.
    pub picked: String,
    /// The pick of a pack in the removed WGSL format that already got its note in the chat.
    pub noted: String,
    /// The pick's `sunrise.hdr` request as last read (`ClientGame::picked_pack_shape`).
    pub pick_hdr: std::cell::RefCell<super::shader_packs::PickHdr>,
    /// The Iris compatibility switch the pack in the renderer (running or building) was loaded
    /// with, and whether it ran with the identity macros; `None` before a load.
    pub identity_build: Option<(IrisCompat, bool)>,
    /// The pick's identity answer as last read: at its load, and each time its files or options
    /// are read again for the Shaders page (`Game::refresh_identity`). `None` before either.
    pub identity_seen: Option<bool>,
    /// The pack list asked for the pick's answer again while its scan was running: a later frame
    /// takes it (`identity_seen`) once the scan has landed.
    pub identity_recheck: bool,
}

impl NativeShaders {
    /// Whether the pack in the renderer was loaded from other inputs than the pick's now: another
    /// switch (`now`), or another identity answer than the last one read (the pack replaced under
    /// its name). It is to be dropped before it can run (a build started before the player turned
    /// it off is never installed: `Renderer::poll_shader_pack_unless`), and is forgotten here.
    pub(super) fn stale_identity(&mut self, now: IrisCompat) -> bool {
        let stale = self.identity_build.is_some_and(|(mode, answer)| {
            mode != now || self.identity_seen.is_some_and(|seen| seen != answer)
        });
        if stale {
            self.identity_build = None;
        }
        stale
    }
}

/// `SUNRISE_HDR_FRAME=float`: the frame is float where the renderer is made with no pack picked,
/// one refused, or one asking for Sunrise's HDR, as HDR without a pack is
/// (`PickedPack::hdr_float`).
pub(in crate::game) fn linear_frame_switch() -> bool {
    std::env::var("SUNRISE_HDR_FRAME").is_ok_and(|v| v == "float")
}

/// A hash of the names, sizes and modification times of the pack's files (`None` when the pack
/// is unreadable).
pub(in crate::game) fn fingerprint(path: &Path) -> Option<u64> {
    fn walk(path: &Path, hasher: &mut std::hash::DefaultHasher) -> Option<()> {
        let meta = std::fs::metadata(path).ok()?;
        path.hash(hasher);
        if meta.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(path)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .collect();
            entries.sort();
            for entry in entries {
                walk(&entry, hasher)?;
            }
        } else {
            meta.len().hash(hasher);
            meta.modified().ok()?.hash(hasher);
        }
        Some(())
    }
    let mut hasher = std::hash::DefaultHasher::new();
    walk(path, &mut hasher)?;
    Some(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::{IrisCompat, fingerprint, follow_device, outcome, refused, remember};
    const ON: IrisCompat = IrisCompat::On;

    #[test]
    fn outcomes_and_refusals_survive_renderer_resets() {
        let root = std::env::temp_dir().join(format!("sunrise-acceptance-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("post.fsh");
        std::fs::write(&file, "first files").unwrap();
        let stamp = fingerprint(&root);
        assert_eq!(outcome(&root, ON), None);
        remember(&root, stamp, ON, true);
        assert_eq!(outcome(&root, ON), Some(true));
        // Policy, SDK and GPU refusals all reach this same outcome before any resource reload.
        remember(&root, stamp, ON, false);
        assert_eq!(outcome(&root, ON), Some(false));
        assert!(refused(&root, fingerprint(&root), ON));
        // The same files with another Iris compatibility switch are tried again.
        assert_eq!(outcome(&root, IrisCompat::Off), None);
        assert!(!refused(&root, fingerprint(&root), IrisCompat::Off));
        assert!(!refused(&root, fingerprint(&root), IrisCompat::Auto));
        std::fs::write(&file, "repaired files with a different size").unwrap();
        assert!(!refused(&root, fingerprint(&root), ON));
        remember(&root, fingerprint(&root), ON, true);
        assert_eq!(outcome(&root, ON), Some(true));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A refusal is forgotten when a renderer on another device is installed, whatever that
    /// device answers, and kept when a renderer is made again on the same device; an acceptance
    /// is kept either way.
    #[test]
    fn a_refusal_is_tried_again_on_another_device() {
        let root = std::env::temp_dir().join(format!("sunrise-device-{}", std::process::id()));
        let (refused_pack, accepted_pack) = (root.join("refused"), root.join("accepted"));
        for pack in [&refused_pack, &accepted_pack] {
            std::fs::create_dir_all(pack).unwrap();
            std::fs::write(pack.join("post.fsh"), "files").unwrap();
        }
        follow_device(7);
        remember(&refused_pack, fingerprint(&refused_pack), ON, false);
        remember(&accepted_pack, fingerprint(&accepted_pack), ON, true);
        // A renderer made again on the same device: nothing to try again.
        follow_device(7);
        assert!(refused(&refused_pack, fingerprint(&refused_pack), ON));
        // Another device: the refusal is tried again, the acceptance stays.
        follow_device(8);
        assert!(!refused(&refused_pack, fingerprint(&refused_pack), ON));
        assert_eq!(outcome(&refused_pack, ON), None);
        assert_eq!(outcome(&accepted_pack, ON), Some(true));
        // Refused on that device too: kept while it stays.
        remember(&refused_pack, fingerprint(&refused_pack), ON, false);
        follow_device(8);
        assert!(refused(&refused_pack, fingerprint(&refused_pack), ON));
        // Back to the first device's number: still another device than the last.
        follow_device(7);
        assert!(!refused(&refused_pack, fingerprint(&refused_pack), ON));
        std::fs::remove_dir_all(root).unwrap();
    }
}
