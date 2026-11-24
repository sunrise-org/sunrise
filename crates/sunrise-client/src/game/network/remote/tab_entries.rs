//! The tab list's entries as the client holds them, for reads that must not change them.

use super::Remote;
use sunrise_client::tab_list::TabPlayer;

impl Remote {
    /// The listed players (`ClientPacketListener.getListedOnlinePlayers`) in encounter order.
    /// `tab_list::sorted_players` gives the overlay's display order.
    pub(in crate::game) fn listed_tab_player_refs(&self) -> impl Iterator<Item = &TabPlayer> {
        self.listed
            .iter()
            .filter_map(|uuid| self.tab_players.get(uuid))
    }
}
