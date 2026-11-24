pub use client::model_instance::{
    GroupError, IDENTITY_TRANSFORM, InstanceError, InstanceGroupContext, InstanceGroupRef,
    LightCoords, ModelInstance, Overlay,
};
// hidden: scoped group access used by InstanceGroupContext.
#[doc(hidden)]
pub use client::model_instance::InstanceGroupBackendRef;
