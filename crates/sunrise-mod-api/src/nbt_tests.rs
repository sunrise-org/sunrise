use mod_api::nbt::{
    JavaString, JsonNbtError, JsonTextError, Tag, from_json, parse_compound_fully, parse_fully,
    to_json, to_snbt,
};

#[test]
fn snbt_full_input_and_utf16_error_cursors() {
    assert_eq!(
        parse_fully(&" \t[B;1B,-2B] \u{2003}".into()),
        Ok(Tag::ByteArray(vec![1, 254]))
    );
    let compound = parse_compound_fully(&"{value:42}\u{3000}".into()).unwrap();
    assert_eq!(compound.get(&"value".into()), Some(&Tag::Int(42)));
    for (source, key, cursor) in [
        ("'😀' \u{2003}x", "argument.nbt.trailing", 6),
        ("'😀'\u{00a0}", "argument.nbt.trailing", 4),
        ("'😀' \t", "argument.nbt.expected.compound", 6),
        ("{a:'😀'", "argument.literal.incorrect", 7),
    ] {
        let err = parse_compound_fully(&source.into()).unwrap_err();
        assert_eq!((err.key, err.cursor), (key, cursor), "{source}");
    }
    // The map-entry action checks the empty key only after reading the value.
    assert_eq!(
        parse_compound_fully(&r#"{"":1}"#.into())
            .unwrap_err()
            .cursor,
        5
    );
}

#[test]
fn compact_snbt_uses_vanilla_key_order_suffixes_and_utf16() {
    let tag = Tag::compound([
        (JavaString(vec![0xe000]), Tag::Long(3)),
        (
            JavaString(vec![0xd800]),
            Tag::String(JavaString(vec![0xdc00, 34, 39, 1, 92])),
        ),
        ("true".into(), Tag::Byte(1)),
        (
            "a".into(),
            Tag::List(vec![
                Tag::Short(-2),
                Tag::Int(5),
                Tag::Float(1.5f32.to_bits()),
                Tag::Double(2.5f64.to_bits()),
                Tag::ByteArray(vec![255]),
                Tag::IntArray(vec![6]),
                Tag::LongArray(vec![7]),
            ]),
        ),
    ]);
    let source = JavaString(
        "{a:[-2s,5,1.5f,2.5d,[B;-1B],[I;6],[L;7L]],\"true\":1b,\""
            .encode_utf16()
            .chain([0xd800])
            .chain("\":'".encode_utf16())
            .chain([0xdc00])
            .chain("\"\\'\\x01\\\\',\"\u{e000}\":3L}".encode_utf16())
            .collect(),
    );
    assert_eq!(to_snbt(&tag), source);
    assert_eq!(parse_fully(&source), Ok(tag));
}

#[test]
fn snbt_walks_beyond_network_nesting_without_recursion() {
    let depth = 8_192;
    let source: JavaString = format!("{}1{}", "[".repeat(depth), "]".repeat(depth))
        .as_str()
        .into();
    let mut tag = parse_fully(&source).unwrap();
    assert_eq!(to_snbt(&tag), source);
    for _ in 0..depth {
        let Tag::List(mut list) = tag else {
            panic!("expected a list");
        };
        assert_eq!(list.len(), 1);
        tag = list.pop().unwrap();
    }
    assert_eq!(tag, Tag::Int(1));
    let deep_child: JavaString = format!("[{},", source.to_string().unwrap()).as_str().into();
    assert_eq!(
        parse_fully(&deep_child).unwrap_err().cursor,
        deep_child.0.len()
    );
    assert_eq!(
        parse_compound_fully(&source).unwrap_err().key,
        "argument.nbt.expected.compound"
    );
    let empty_key: JavaString = format!("{{\"\":{}}}", source.to_string().unwrap())
        .as_str()
        .into();
    let error = parse_fully(&empty_key).unwrap_err();
    assert_eq!(error.cursor, empty_key.0.len() - 1);
}

#[test]
fn json_input_keeps_member_order_and_narrows_the_original_number() {
    let source = r#"{"BB":null,"Aa":false,"BB":[true,128,32768,2147483648,9007199254740993.0,1.5,0.1,"text"]}"#;
    let Tag::Compound(tag) = from_json(source).unwrap() else {
        panic!("expected compound");
    };
    assert_eq!(
        tag.keys().cloned().collect::<Vec<_>>(),
        [JavaString::from("BB"), JavaString::from("Aa")]
    );
    assert_eq!(tag.get(&"Aa".into()), Some(&Tag::Byte(0)));
    assert_eq!(
        tag.get(&"BB".into()),
        Some(&Tag::List(vec![
            Tag::Byte(1),
            Tag::Short(128),
            Tag::Int(32768),
            Tag::Long(2147483648),
            Tag::Long(9007199254740993),
            Tag::Float(1.5f32.to_bits()),
            Tag::Double(0.1f64.to_bits()),
            Tag::String("text".into()),
        ]))
    );
    assert_eq!(from_json("null"), Ok(Tag::End));
    assert_eq!(from_json("1e400"), Ok(Tag::Float(f32::INFINITY.to_bits())));
    assert_eq!(
        from_json(r#"{"\uD800":"\uDC00"}"#),
        Ok(Tag::compound([(
            JavaString(vec![0xd800]),
            Tag::String(JavaString(vec![0xdc00])),
        )]))
    );
}

#[test]
fn json_duplicate_members_discard_conversion_errors_but_not_syntax_errors() {
    let expected = Ok(Tag::compound([("x", Tag::Byte(1))]));
    for source in [r#"{"x":1e10000,"x":1}"#, r#"{"x":[{"y":1e10000}],"x":1}"#] {
        assert_eq!(from_json(source), expected);
    }
    assert_eq!(
        from_json(r#"{"x":1,"x":1e10000}"#),
        Err(JsonTextError::NumberOutOfRange { cursor: 11 })
    );
    assert!(matches!(
        from_json(r#"{"x":1e,"x":1}"#),
        Err(JsonTextError::Malformed { .. })
    ));
    let deep = format!("{}1{}", "[".repeat(512), "]".repeat(512));
    assert_eq!(from_json(&format!(r#"{{"x":{deep},"x":1}}"#)), expected);
}

#[test]
fn json_output_uses_numbers_lists_and_null_without_type_wrappers() {
    let tag = Tag::List(vec![
        Tag::compound([
            ("BB", Tag::String("line\nbreak".into())),
            ("Aa", Tag::Int(9)),
        ]),
        Tag::Byte(1),
        Tag::Short(2),
        Tag::Long(3),
        Tag::Float(0.1f32.to_bits()),
        Tag::ByteArray(vec![255]),
        Tag::IntArray(vec![4]),
        Tag::LongArray(vec![5]),
        Tag::End,
    ]);
    assert_eq!(
        to_json(&tag).unwrap(),
        r#"[{"BB":"line\nbreak","Aa":9},1,2,3,0.1,[-1],[4],[5],null]"#
    );
}

#[test]
fn json_refuses_malformed_input_and_unrepresentable_values_by_name() {
    for source in [
        r#"{"x" 1}"#,
        "[1,]",
        "true false",
        "1e",
        "01",
        r#""\uDG00""#,
    ] {
        assert!(
            matches!(from_json(source), Err(JsonTextError::Malformed { .. })),
            "{source}"
        );
    }
    assert_eq!(
        from_json("1e10000"),
        Err(JsonTextError::NumberOutOfRange { cursor: 0 })
    );
    assert_eq!(
        from_json("[null]"),
        Err(JsonTextError::Conversion(JsonNbtError::NestedNull))
    );
    assert_eq!(
        to_json(&Tag::Double(f64::NAN.to_bits())),
        Err(JsonNbtError::NonFinite)
    );
    for tag in [
        Tag::String(JavaString(vec![0xd800])),
        Tag::compound([(JavaString(vec![0xdc00]), Tag::Byte(0))]),
    ] {
        assert_eq!(to_json(&tag), Err(JsonNbtError::InvalidString));
    }
}

#[test]
fn json_depth_is_bounded_on_both_text_paths() {
    let source = format!("{}1{}", "[".repeat(512), "]".repeat(512));
    let tag = from_json(&source).unwrap();
    assert_eq!(to_json(&tag).unwrap(), source);
    assert_eq!(
        from_json(&format!("[{source}]")),
        Err(JsonTextError::Conversion(JsonNbtError::TooDeep))
    );
    assert_eq!(to_json(&Tag::List(vec![tag])), Err(JsonNbtError::TooDeep));
}
