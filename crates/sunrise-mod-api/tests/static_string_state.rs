mod tests {
    use mod_api::wire::{Reader, WireError, Writer};
    #[test]
    fn static_string_rows_roundtrip_through_intern() {
        mod_api::__private::initialize().unwrap();
        let rows: Vec<(&'static str, &'static str)> =
            vec![("test:block", "test:glow"), ("test:block", "test:shade")];
        let mut out = Writer::new();
        out.put(&rows);
        let decoded = Reader::decode::<Vec<(&'static str, &'static str)>>(out.bytes()).unwrap();
        assert_eq!(decoded, rows);
        assert!(std::ptr::eq(decoded[0].0.as_ptr(), decoded[1].0.as_ptr()));
        let mut oversized = Writer::new();
        oversized.put(&"x".repeat(257));
        assert_eq!(
            Reader::decode::<&'static str>(oversized.bytes()),
            Err(WireError::Invalid("static string too long"))
        );
    }
}
