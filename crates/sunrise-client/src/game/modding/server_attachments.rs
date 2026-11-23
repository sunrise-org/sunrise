//! The server's own attachment holder, `Holder::Server`: one map for the whole world that every
//! level's hooks reach, kept where vanilla keeps the world's other saved data (the overworld's
//! `data/` folder, beside the scoreboard and command storage) as `data/sunrise/server_attachments.dat`.
//!
//! Only the logical server has it. It is read on first use, written with the world's other data,
//! journaled like any holder's map (`journal_attachments`), and never synced to clients.
use super::Game;
use mod_api::{AttachmentError, Attachments};

/// The world's server-wide attachments, read from the save on first use.
#[derive(Default)]
pub(super) struct ServerAttachments {
    map: Attachments,
    loaded: bool,
}

impl ServerAttachments {
    /// The map, read from the world at `root` the first time.
    pub(super) fn map(&mut self, root: &std::path::Path) -> &mut Attachments {
        if !self.loaded {
            self.loaded = true;
            self.map = sunrise_client::save::load_attachments_file(&path(root));
        }
        &mut self.map
    }
}

fn path(root: &std::path::Path) -> std::path::PathBuf {
    root.join("data")
        .join("sunrise")
        .join("server_attachments.dat")
}

impl Game<'_> {
    /// The server's map. A client has none: `MissingHolder`.
    pub(super) fn server_attachment_map(&mut self) -> Result<&mut Attachments, AttachmentError> {
        if !self.native_authoritative() || self.remote.is_some() {
            return Err(AttachmentError::MissingHolder);
        }
        Ok(self.server_attachments.map(&self.world_root))
    }

    /// Write the server's map with the world's other data; nothing if it was never read.
    pub(super) fn save_server_attachments(&mut self) {
        if !self.server_attachments.loaded || self.remote.is_some() {
            return;
        }
        if let Err(e) = sunrise_client::save::save_attachments_file(
            &path(&self.world_root),
            &self.server_attachments.map,
            false,
        ) {
            log::warn!("the server's mod attachments were not written: {e}");
        }
    }
}
