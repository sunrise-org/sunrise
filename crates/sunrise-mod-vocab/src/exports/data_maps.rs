pub use world::data_maps::{
    DataMap, DataMapEntry, DataMapError, DataMapSync, RawDataMap, data_map_entries, data_map_value,
};

mod __private {
    pub use crate::world::data_maps::RegisteredDataMap;
}
