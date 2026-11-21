impl EntityRef {
    fn kind<H: ?Sized>(self, _: &H) -> Option<u32> {
        Some(0)
    }
}
