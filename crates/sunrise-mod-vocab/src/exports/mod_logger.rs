pub use crate::world::mod_logger::{LogLevel, LogOutcome, ModLogger};
mod __private {
    pub mod game {
        pub use crate::world::mod_logger::install_mod_log_sink;
    }
}
