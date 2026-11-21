pub use client::special_model_renderers::{
    MAX_NESTED_STACK_PARTS, ReplacedSpecialModelRenderer, SpecialModelBakeContext,
    SpecialModelCollector, SpecialModelConfig, SpecialModelContext, SpecialModelFactory,
};
pub use client::special_model_renderers::{
    SpecialModelGeometry, SpecialModelQuad, SpecialModelRenderer, SpecialModelRendererRegistry,
    SpecialModelRenderers, SpecialModelStack, SpecialModelTints,
};
mod __private {
    pub use crate::client::special_model_renderers::{
        BakedSpecialModelRenderer, SpecialModelSubmission,
    };
}
