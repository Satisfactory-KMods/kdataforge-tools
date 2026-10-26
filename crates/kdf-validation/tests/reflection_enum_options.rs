use kdf_validation::{slots::Slot, PropertyInfo};
use serde_json::{json, Value};

fn property(kind: &str, mode: &str, bytes: u64, options: Value) -> Value {
    json!({
        "name":"Flags", "kind":kind, "cppType":"ETestFlags",
        "enum":"/Script/Test.ETestFlags", "bitflags":true,
        "bitmaskMode":mode, "elementSize":bytes, "options":options
    })
}

#[test]
fn enum_mask_choices_respect_storage_width_and_remove_negative_masks() {
    for kind in ["EnumProperty", "ByteProperty"] {
        let info = PropertyInfo::from_record(&property(
            kind,
            "maskValues",
            1,
            json!([
                {"name":"None","valueExact":"0"},
                {"name":"Low","valueExact":"1"},
                {"name":"High","valueExact":"128"},
                {"name":"Overflow","valueExact":"256"},
                {"name":"Negative","valueExact":"-1"},
                {"name":"ETestFlags_MAX","isMax":true,"valueExact":"255"}
            ]),
        ));
        assert_eq!(info.enum_options, ["None", "Low", "High"], "{kind}");
        assert_eq!(
            Slot::from_property(&info),
            Slot::Enum(info.enum_options.clone())
        );
    }
}

#[test]
fn enum_bit_indices_validate_raw_ordinals_instead_of_converting_names_to_bits() {
    let info = PropertyInfo::from_record(&property(
        "EnumProperty",
        "bitIndices",
        1,
        json!([
            {"name":"Zero","valueExact":"0"},
            {"name":"Four","valueExact":"4"}
        ]),
    ));
    // Declarations permit bits 0 and 4 (mask 17). Named Four writes ordinal 4,
    // which contains undeclared bit 2, so it cannot be offered as a valid value.
    assert_eq!(info.enum_options, ["Zero"]);

    let info = PropertyInfo::from_record(&property(
        "ByteProperty",
        "bitIndices",
        1,
        json!([
            {"name":"Six","valueExact":"6"},
            {"name":"SixtyFour","valueExact":"64"}
        ]),
    ));
    // Ordinal 64 is valid because index 6 declares bit 64. Enum-backed names
    // do not have the numeric BitmaskEnum index < 64 restriction.
    assert_eq!(info.enum_options, ["SixtyFour"]);
}

#[test]
fn numeric_bitmask_choices_convert_indices_and_keep_full_storage_sign_bit() {
    let options = json!([
        {"name":"Low","valueExact":"0"},
        {"name":"Sign","valueExact":"63"},
        {"name":"InvalidIndex","valueExact":"64"},
        {"name":"Negative","valueExact":"-1"}
    ]);
    let info = PropertyInfo::from_record(&property("Int64Property", "bitIndices", 8, options));
    assert_eq!(info.enum_options, ["Low", "Sign"]);
    assert!(matches!(Slot::from_property(&info), Slot::Int(_)));
    let info = PropertyInfo::from_record(&property(
        "Int8Property",
        "bitIndices",
        1,
        json!([
            {"name":"Sign","valueExact":"7"},
            {"name":"Overflow","valueExact":"8"}
        ]),
    ));
    assert_eq!(info.enum_options, ["Sign"]);
}

#[test]
fn numeric_mask_choices_preserve_exact_values_and_enforce_32_bit_boundary() {
    let info = PropertyInfo::from_record(&property(
        "IntProperty",
        "maskValues",
        4,
        json!([
            {"name":"Sign","valueExact":"2147483648"},
            {"name":"Maximum","valueExact":"4294967295"},
            {"name":"Overflow","valueExact":"4294967296"},
            {"name":"ExactNegative","valueExact":"-1","value":1}
        ]),
    ));
    assert_eq!(info.enum_options, ["Sign", "Maximum"]);
    let info = PropertyInfo::from_record(&property(
        "EnumProperty",
        "maskValues",
        8,
        json!([
            {"name":"BeyondJsInteger","valueExact":"9007199254740993","value":9007199254740992_u64},
            {"name":"ExactPositive","valueExact":"1","value":-1}
        ]),
    ));
    assert_eq!(info.enum_options, ["BeyondJsInteger", "ExactPositive"]);
}

#[test]
fn known_mode_still_checks_declared_bits_without_property_width() {
    let mut field = property(
        "EnumProperty",
        "bitIndices",
        1,
        json!([
            {"name":"Zero","valueExact":"0"},
            {"name":"Four","valueExact":"4"}
        ]),
    );
    field.as_object_mut().unwrap().remove("elementSize");
    assert_eq!(PropertyInfo::from_record(&field).enum_options, ["Zero"]);
    field["elementSize"] = json!(3);
    assert_eq!(PropertyInfo::from_record(&field).enum_options, ["Zero"]);
}

#[test]
fn absent_mode_preserves_exact_members_including_negative_values() {
    let mut field = property(
        "EnumProperty",
        "maskValues",
        1,
        json!([
            {"name":"Negative","valueExact":"-1"},
            {"name":"Declared","valueExact":"512"}
        ]),
    );
    field.as_object_mut().unwrap().remove("bitmaskMode");
    assert_eq!(
        PropertyInfo::from_record(&field).enum_options,
        ["Negative", "Declared"]
    );
}

#[test]
fn incomplete_numeric_metadata_keeps_names_without_inventing_invalidity() {
    let info = PropertyInfo::from_record(&property(
        "EnumProperty",
        "maskValues",
        1,
        json!([
            {"name":"Unknown"},
            {"name":"BrokenExact","valueExact":"unknown","value":-1}
        ]),
    ));
    assert_eq!(info.enum_options, ["Unknown", "BrokenExact"]);
}

#[test]
fn ordinary_enums_keep_declared_choices() {
    let mut field = property(
        "EnumProperty",
        "maskValues",
        1,
        json!([
            {"name":"Negative","valueExact":"-1"},
            {"name":"Large","valueExact":"512"}
        ]),
    );
    field["bitflags"] = json!(false);
    assert_eq!(
        PropertyInfo::from_record(&field).enum_options,
        ["Negative", "Large"]
    );
}
