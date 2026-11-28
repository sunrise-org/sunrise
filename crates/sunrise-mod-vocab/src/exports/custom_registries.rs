pub use world::custom_registries::{
    CustomRegistry, EntryKey, HostRegistries, RegistryKey, RegistryOptions,
};

mod __private {
    pub use crate::world::custom_registries::{
        CustomRegistryFreezer, FrozenCustomRegistries, RegistryDirectory,
    };
}
