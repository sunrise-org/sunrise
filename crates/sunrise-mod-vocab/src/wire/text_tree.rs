//! Components cross as a private, validated node table, not Rust recursion.
use super::{Reader, Wire, WireError, Writer};
use crate::text::{Text, TextForm};

impl Wire for Text {
    const MIN: usize = TextForm::MIN;
    layout_via!(TextForm, "validated-text-node-table:v1", ordinary_only);
    fn put(&self, out: &mut Writer) {
        // Rebuilding the tree from its node table cannot charge the payload budget.
        if !out.ordinary_only() {
            return;
        }
        match TextForm::from_text(self) {
            Ok(form) => out.put(&form),
            Err(_) => out.fail(WireError::Depth),
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        input.ordinary_only()?;
        input
            .take::<TextForm>()?
            .into_text()
            .map_err(|_| WireError::Invalid("text node table"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{TextHover, TextStyle};

    #[test]
    fn structured_text_keeps_existing_wire_bytes() {
        const EMPTY_COMPOUND: &[u8] = &[10, 0, 0, 0, 0, 0, 0, 0];
        let text = Text::translate("k", vec![Text::literal("a")])
            .append(Text::literal("c"))
            .with_style(TextStyle {
                bold: Some(false),
                hover: Some(TextHover::Text(Box::new(Text::literal("h")))),
                ..TextStyle::default()
            });

        let mut bytes = vec![4, 0, 0, 0]; // Four rows, in traversal order.
        bytes.extend([0, 0, 0, 0, 0]); // No parent; root has the child relation.
        bytes.extend([1, 0, 0, 0, 1, 0, 0, 0, b'k', 0]); // Translate; key; no fallback.
        bytes.extend([0, 0, 1, 0, 0, 0, 0, 0, 0]); // Bold is explicitly false.
        bytes.extend([1, 0, 0, 0, 0, 0, 0]); // Text hover marker; no insertion or font.
        bytes.extend_from_slice(EMPTY_COMPOUND); // Style metadata.
        bytes.extend_from_slice(EMPTY_COMPOUND); // Component metadata.
        for (relation, letter) in [(0, b'c'), (1, b'a'), (4, b'h')] {
            bytes.extend([1, 0, 0, 0, 0, relation, 0, 0, 0]); // Parent row 0; relation.
            bytes.extend([0, 0, 0, 0, 1, 0, 0, 0, letter]); // Literal of length one.
            bytes.extend([0; 11]); // No style overrides.
            bytes.extend_from_slice(EMPTY_COMPOUND);
            bytes.extend_from_slice(EMPTY_COMPOUND);
        }
        let mut out = Writer::new();
        out.put(&text);
        assert_eq!(out.error(), None);
        assert_eq!(out.bytes(), bytes.as_slice());
        assert_eq!(Reader::decode::<Text>(&bytes).unwrap(), text);
    }
}
