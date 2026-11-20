pub struct HiddenLayers<'a>(std::marker::PhantomData<&'a ()>);

impl HiddenLayers<'_> {
    pub(crate) fn guard<T, E>(&self, call: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        call()
    }
}
