//! Read-only serializers share registration and native values with crafting serializers.
use super::*;
/// A JSON serializer for a separately registered recipe type that the mod reads.
///
/// Use `register_read_only_recipe_serializer` in `Hooks::register`. The game
/// publishes the source JSON through `recipes_of_type` and never matches it.
/// The loader retains each decoded value with its owning serializer for the
/// recipe generation. The value stays in process and needs no `Wire` implementation.
pub trait RecipeSerializer: Send + Sync + 'static {
    /// Mod-owned recipe data decoded once per generation and borrowed when writing JSON.
    type Value: Send + Sync + 'static;
    /// Decode a recipe JSON object with its `type` field removed.
    /// Return an error to skip a malformed recipe without refusing other files.
    fn decode(&self, json: &str) -> Result<Self::Value, String>;
    /// Write an equivalent JSON object for Sunrise client synchronization.
    /// The game supplies the registered serializer ID as `type` and applies the packet limits.
    fn write(&self, value: &Self::Value) -> Result<String, String>;
}
struct ReadOnly<S>(S);
impl<S: RecipeSerializer> crate::CraftingRecipeKind for ReadOnly<S> {
    type Value = S::Value;
    fn decode(&self, json: &str) -> Result<Self::Value, String> {
        self.0.decode(json)
    }
    fn write(&self, value: &Self::Value) -> Result<String, String> {
        self.0.write(value)
    }
    fn matches(&self, _: &dyn crate::LoopContext, _: &Self::Value, _: crate::CraftingGrid) -> bool {
        false
    }
    fn assemble(
        &self,
        _: &dyn crate::LoopContext,
        _: &Self::Value,
        _: crate::CraftingGrid,
    ) -> Option<crate::ItemStack> {
        None
    }
}
struct Raw;
impl RecipeSerializer for Raw {
    type Value = String;
    fn decode(&self, json: &str) -> Result<String, String> {
        // The loader supplies a compact object with `type` removed. Keep the old
        // string limit: 1 MiB minus the four bytes of its former length prefix.
        const MAX_RAW_RECIPE_JSON_BYTES: usize = 1024 * 1024 - 4;
        if json.len() > MAX_RAW_RECIPE_JSON_BYTES {
            return Err(format!(
                "raw recipe JSON exceeds {MAX_RAW_RECIPE_JSON_BYTES} bytes"
            ));
        }
        Ok(json.to_owned())
    }
    fn write(&self, value: &String) -> Result<String, String> {
        Ok(value.clone())
    }
}
impl dyn Registrar + '_ {
    /// Register a serializer for an independently declared read-only recipe type.
    pub fn register_read_only_recipe_serializer<S: RecipeSerializer>(
        &mut self,
        name: &str,
        recipe_type: &str,
        serializer: S,
    ) {
        let kind = ReadOnly(serializer);
        let handler = crate::parts::crafting_recipes::typed_handler(kind);
        self.add_recipe_serializer(name, recipe_type, handler);
    }
    /// Register an object-preserving JSON serializer for a mod-read machine recipe type.
    /// The compact JSON object without `type` is limited to 1 MiB minus four bytes.
    /// Declare `recipe_type` with `register_recipe_type` at startup, in either order.
    pub fn register_raw_recipe_serializer(&mut self, name: &str, recipe_type: &str) {
        self.register_read_only_recipe_serializer(name, recipe_type, Raw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_recipe_accepts_the_previous_json_limit_but_not_one_byte_more() {
        let padding = "x".repeat(1_048_572 - r#"{"value":""}"#.len());
        let at_limit = format!(r#"{{"value":"{padding}"}}"#);
        assert_eq!(Raw.decode(&at_limit).unwrap(), at_limit);
        let over_limit = format!(r#"{{"value":"{padding}x"}}"#);
        assert!(Raw.decode(&over_limit).is_err());
    }
}
