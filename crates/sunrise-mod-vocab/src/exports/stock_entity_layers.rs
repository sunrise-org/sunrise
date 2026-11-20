pub use client::stock_entity_layers::{
    ArmorLayerPass, ArmorLayerSlot, HiddenLayers, HideLayerError, StockEntityLayer,
};
// hidden: only the renderer constructs and reads the fixed-size mask.
#[doc(hidden)]
pub use client::stock_entity_layers::StockLayerMask;
