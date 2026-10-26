use kdf_validation::{lint_path, LintOptions, LintSettings, Profile, ReflectionIndex};
use std::{fs, io::Write, sync::Arc};

fn fixture() -> (
    tempfile::TempDir,
    tempfile::NamedTempFile,
    Arc<ReflectionIndex>,
) {
    fixture_with_integer_fields(false)
}

fn fixture_with_integer_fields(
    include_integers: bool,
) -> (
    tempfile::TempDir,
    tempfile::NamedTempFile,
    Arc<ReflectionIndex>,
) {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("pack.yml"),
        "ref: Example\nname: Example\nversion: 1.0.0\n",
    )
    .unwrap();
    let archive = tempfile::Builder::new()
        .suffix(".kdfdata")
        .tempfile()
        .unwrap();
    let mut zip = zip::ZipWriter::new(fs::File::create(archive.path()).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", options).unwrap();
    write!(
        zip,
        "{}",
        serde_json::json!({
            "schemaVersion":1,"metadataAvailable":true,
            "counts":{"classes":if include_integers { 4 } else { 3 }},"failedLoads":[]
        })
    )
    .unwrap();
    zip.start_file("classes.ndjson", options).unwrap();
    if include_integers {
        writeln!(
            zip,
            "{}",
            serde_json::json!({
                "path":"/Script/Test.Integers",
                "properties":[
                    {"name":"mUnsigned","cppType":"uint64","kind":"UInt64Property"},
                    {"name":"mSigned","cppType":"int64","kind":"Int64Property"},
                    {"name":"mSigned32","cppType":"int32","kind":"IntProperty"},
                    {"name":"mByte","cppType":"uint8","kind":"ByteProperty"},
                    {"name":"mFlags8","cppType":"int8","kind":"Int8Property","bitflags":true,"elementSize":1},
                    {"name":"mFlags16","cppType":"int16","kind":"Int16Property","bitflags":true,"elementSize":2},
                    {"name":"mFlags32","cppType":"int32","kind":"IntProperty","bitflags":true,"elementSize":4},
                    {"name":"mFlags64","cppType":"int64","kind":"Int64Property","bitflags":true,"elementSize":8},
                    {"name":"mFlagsUnsigned64","cppType":"uint64","kind":"UInt64Property","bitflags":true,"elementSize":8},
                    {"name":"mNamedFlags32","cppType":"int32","kind":"IntProperty","bitflags":true,"elementSize":4,
                     "options":[{"name":"A","fullName":"TestFlags::A","valueExact":"0"},{"name":"B","fullName":"TestFlags::B","valueExact":"1"}]},
                    {"name":"mMaskValues32","cppType":"int32","kind":"IntProperty","bitflags":true,"elementSize":4,"bitmaskMode":"maskValues",
                     "options":[{"name":"Low","valueExact":"1"},{"name":"Sign","valueExact":"2147483648"},{"name":"EFlags_MAX","valueExact":"4294967295"},{"name":"Negative","valueExact":"-1"}]},
                    {"name":"mBitIndices64","cppType":"int64","kind":"Int64Property","bitflags":true,"elementSize":8,"bitmaskMode":"bitIndices",
                     "options":[{"name":"Low","valueExact":"0"},{"name":"Sign","valueExact":"63"},{"name":"EFlags_MAX","valueExact":"62"},{"name":"Negative","valueExact":"-1"},{"name":"InvalidIndex","valueExact":"64"}]},
                    {"name":"mBareMax","cppType":"int8","kind":"Int8Property","bitflags":true,"elementSize":1,"bitmaskMode":"maskValues",
                     "options":[{"name":"MAX","isMax":true,"valueExact":"4"}]},
                    {"name":"mNarrowFlags","cppType":"int8","kind":"Int8Property","bitflags":true,"elementSize":1,"bitmaskMode":"bitIndices",
                     "options":[{"name":"Low","valueExact":"0"},{"name":"OutOfWidth","valueExact":"8"}]},
                    {"name":"mExactMask64","cppType":"int64","kind":"Int64Property","bitflags":true,"elementSize":8,"bitmaskMode":"maskValues",
                     "options":[{"name":"Large","valueExact":"9007199254740993","value":9007199254740992_u64}]},
                    {"name":"mEnumMaskValues","cppType":"EEnumMaskValues","kind":"EnumProperty","enum":"/Script/Test.EEnumMaskValues","bitflags":true,"elementSize":4,"bitmaskMode":"maskValues",
                     "options":[{"name":"Low","fullName":"EEnumMaskValues::Low","valueExact":"1"},{"name":"High","fullName":"EEnumMaskValues::High","valueExact":"2147483648"},{"name":"EEnumMaskValues_MAX","valueExact":"2147483649"}]},
                    {"name":"mByteBitIndices","cppType":"uint8","kind":"ByteProperty","enum":"/Script/Test.EByteBitIndices","bitflags":true,"elementSize":1,"bitmaskMode":"bitIndices",
                     "options":[{"name":"Low","fullName":"EByteBitIndices::Low","valueExact":"0"},{"name":"High","fullName":"EByteBitIndices::High","valueExact":"1"}]},
                    {"name":"mEnumNoWidth","cppType":"EFlags","kind":"EnumProperty","enum":"/Script/Test.EFlags","bitflags":true,"bitmaskMode":"bitIndices",
                     "options":[{"name":"Zero","valueExact":"0"},{"name":"Four","valueExact":"4"}]},
                    {"name":"mEnumNoMode","cppType":"EFlags","kind":"EnumProperty","enum":"/Script/Test.EFlags","bitflags":true,"elementSize":1,
                     "options":[{"name":"Negative","valueExact":"-1"},{"name":"Two","valueExact":"2"}]},
                    {"name":"mEnumUnknownValue","cppType":"EFlags","kind":"EnumProperty","enum":"/Script/Test.EFlags","bitflags":true,
                     "options":[{"name":"Known","valueExact":"2"},{"name":"Unknown"}]}
                ]
            })
        )
        .unwrap();
    }
    zip.write_all(concat!(
        "{\"path\":\"/Script/Test.Base\",\"properties\":[{\"name\":\"mFlag\",\"cppType\":\"bool\",\"kind\":\"BoolProperty\"},{\"name\":\"mDisplayName\",\"cppType\":\"FText\",\"kind\":\"TextProperty\"}]}\n",
        "{\"path\":\"/Script/Test.Child\",\"super\":\"/Script/Test.Base\",\"properties\":[{\"name\":\"mCount\",\"cppType\":\"int32\",\"kind\":\"IntProperty\"},{\"name\":\"mCounts\",\"cppType\":\"TArray<int32>\",\"kind\":\"ArrayProperty\",\"inner\":{\"name\":\"mCounts\",\"cppType\":\"int32\",\"kind\":\"IntProperty\"}},{\"name\":\"mMode\",\"cppType\":\"EResourceForm\",\"enum\":\"/Script/Test.EResourceForm\",\"options\":[{\"name\":\"RF_SOLID\",\"fullName\":\"EResourceForm::RF_SOLID\",\"valueExact\":\"0\",\"value\":0}]}]}\n",
        "{\"path\":\"/Script/Test.Limited\",\"super\":\"/Script/NotExported.Base\",\"properties\":[]}\n"
    ).as_bytes()).unwrap();
    zip.finish().unwrap();
    let index = Arc::new(ReflectionIndex::load(archive.path()));
    (directory, archive, index)
}

fn configured_options(
    directory: &std::path::Path,
    index: Arc<ReflectionIndex>,
    config: &str,
) -> LintOptions {
    fs::write(directory.join(".kdf-editor.toml"), config).unwrap();
    let mut options = LintSettings::load(directory, None)
        .unwrap()
        .lint_options(Profile::Runtime);
    options.datapack_index = Some(index);
    options
}

#[test]
fn unsigned_integer_fields_preserve_exact_u64_boundaries_for_numbers_and_decimal_strings() {
    let (directory, _archive, index) = fixture_with_integer_fields(true);
    let selected = directory.path().join("integers.cdo.yml");
    let cases = [
        ("mUnsigned", "0", true),
        ("mUnsigned", "9223372036854775808", true),
        ("mUnsigned", "18446744073709551615", true),
        ("mUnsigned", "18446744073709551616", false),
        ("mUnsigned", "-1", false),
        ("mSigned", "9223372036854775807", true),
        ("mSigned", "-9223372036854775808", true),
        ("mSigned", "9223372036854775808", false),
        ("mSigned", "-9223372036854775809", false),
        ("mSigned32", "2147483647", true),
        ("mSigned32", "2147483648", false),
        ("mSigned32", "-2147483648", true),
        ("mSigned32", "-2147483649", false),
        ("mByte", "255", true),
        ("mByte", "256", false),
        ("mByte", "-1", false),
    ];
    let options = configured_options(directory.path(), index, "");
    for (field, value, valid) in cases {
        for quoted in [false, true] {
            let scalar = if quoted {
                format!("'{value}'")
            } else {
                value.to_owned()
            };
            fs::write(&selected, format!(
                "type: cdo\npatches:\n  - target: /Script/Test.Integers\n    properties:\n      - path: {field}\n        value: {scalar}\n"
            )).unwrap();
            let report = lint_path(&selected, &options).unwrap();
            // serde_yaml rejects raw integers outside JSON's signed/unsigned ranges before
            // reflection validation; quoted decimal strings reach the exact integer checks.
            let rejected = report.diagnostics.iter().any(|item| {
                item.code == "datapack.type-mismatch" || (!quoted && item.code == "yaml.syntax")
            });
            assert_eq!(
                rejected, !valid,
                "field={field}, scalar={scalar}, diagnostics={:?}",
                report.diagnostics
            );
        }
    }
}

#[test]
fn raw_bitmask_sets_use_exact_unsigned_storage_width_even_for_signed_fields() {
    let (directory, _archive, index) = fixture_with_integer_fields(true);
    let selected = directory.path().join("bitmasks.cdo.yml");
    let options = configured_options(directory.path(), index, "");
    let cases = [
        ("mFlags8", "128", true),
        ("mFlags8", "255", true),
        ("mFlags8", "256", false),
        ("mFlags16", "32768", true),
        ("mFlags16", "65535", true),
        ("mFlags16", "65536", false),
        ("mFlags32", "2147483648", true),
        ("mFlags32", "4294967295", true),
        ("mFlags32", "4294967296", false),
        ("mFlags64", "9223372036854775808", true),
        ("mFlags64", "18446744073709551615", true),
        ("mFlags64", "18446744073709551616", false),
        ("mFlagsUnsigned64", "18446744073709551615", true),
        ("mFlagsUnsigned64", "18446744073709551616", false),
        ("mFlags32", "0", true),
        ("mFlags32", "-1", false),
        ("mFlags64", "-9223372036854775808", false),
        ("mFlags64", "1.5", false),
        ("mFlags64", ".5", false),
        ("mFlags64", "18446744073709551615.0", false),
    ];
    for (field, value, valid) in cases {
        for quoted in [false, true] {
            let scalar = if quoted {
                format!("'{value}'")
            } else {
                value.to_owned()
            };
            fs::write(&selected, format!(
                "type: cdo\npatches:\n  - target: /Script/Test.Integers\n    properties:\n      - path: {field}\n        op: set\n        value: {scalar}\n"
            )).unwrap();
            let report = lint_path(&selected, &options).unwrap();
            // Unquoted integers exceeding u64 may fail YAML decoding first.
            let rejected = report.diagnostics.iter().any(|item| {
                item.code == "datapack.type-mismatch" || (!quoted && item.code == "yaml.syntax")
            });
            assert_eq!(
                rejected, !valid,
                "field={field}, scalar={scalar}, diagnostics={:?}",
                report.diagnostics
            );
        }
    }
}

#[test]
fn raw_bitmask_sets_reject_undeclared_bits_when_exported_mode_is_known() {
    let (directory, _archive, index) = fixture_with_integer_fields(true);
    let selected = directory.path().join("bitmasks.cdo.yml");
    let options = configured_options(directory.path(), index, "");
    let cases = [
        ("mMaskValues32", "0", true),
        ("mMaskValues32", "2147483648", true),
        ("mMaskValues32", "2147483649", true),
        ("mMaskValues32", "2", false),
        ("mMaskValues32", "4294967295", false),
        ("mMaskValues32", "4294967296", false),
        ("mBitIndices64", "9223372036854775808", true),
        ("mBitIndices64", "9223372036854775809", true),
        ("mBitIndices64", "2", false),
        ("mBitIndices64", "4611686018427387904", false),
        ("mBitIndices64", "18446744073709551615", false),
        // Runtime excludes only the _MAX suffix, not bare MAX/isMax alone.
        ("mBareMax", "4", true),
        ("mBareMax", "1", false),
        ("mExactMask64", "9007199254740993", true),
        ("mExactMask64", "9007199254740992", true),
        ("mExactMask64", "1", true),
        ("mExactMask64", "2", false),
        // Legacy metadata has no mode: width validation remains conservative.
        ("mNamedFlags32", "3", true),
    ];
    for (field, value, valid) in cases {
        for quoted in [false, true] {
            let scalar = if quoted {
                format!("'{value}'")
            } else {
                value.to_owned()
            };
            fs::write(&selected, format!(
                "type: cdo\npatches:\n  - target: /Script/Test.Integers\n    properties:\n      - path: {field}\n        op: set\n        value: {scalar}\n"
            )).unwrap();
            let report = lint_path(&selected, &options).unwrap();
            let rejected = report
                .diagnostics
                .iter()
                .any(|item| item.code == "datapack.type-mismatch");
            assert_eq!(
                rejected, !valid,
                "field={field}, scalar={scalar}, diagnostics={:?}",
                report.diagnostics
            );
        }
    }
}

#[test]
fn enum_backed_bitflags_follow_runtime_single_value_and_declared_mask_rules() {
    let (directory, _archive, index) = fixture_with_integer_fields(true);
    let selected = directory.path().join("enum-bitmasks.cdo.yml");
    let options = configured_options(directory.path(), index, "");
    let cases = [
        ("mEnumMaskValues", "2147483648", true),
        ("mEnumMaskValues", "2147483649", true),
        ("mEnumMaskValues", "2", false),
        ("mEnumMaskValues", "'High'", true),
        ("mEnumMaskValues", "'Missing'", false),
        ("mEnumMaskValues", "'Low|High'", false),
        ("mEnumMaskValues", "[Low, High]", false),
        ("mByteBitIndices", "1", true),
        ("mByteBitIndices", "3", true),
        ("mByteBitIndices", "4", false),
        ("mByteBitIndices", "'High'", true),
        ("mEnumNoWidth", "'Zero'", true),
        ("mEnumNoWidth", "'Four'", false),
        ("mEnumNoWidth", "4", false),
        ("mEnumNoWidth", "16", true),
        ("mEnumNoMode", "'Negative'", true),
        ("mEnumNoMode", "-1", true),
        ("mEnumNoMode", "'Two'", true),
        ("mEnumNoMode", "'Missing'", false),
        ("mEnumNoMode", "3", false),
        ("mEnumNoMode", "-2", false),
        ("mEnumUnknownValue", "'Unknown'", true),
        ("mEnumUnknownValue", "3", true),
    ];
    for (field, value, valid) in cases {
        fs::write(
            &selected,
            format!(
                "type: cdo\npatches:\n  - target: /Script/Test.Integers\n    properties:\n      - path: {field}\n        op: set\n        value: {value}\n"
            ),
        )
        .unwrap();
        let report = lint_path(&selected, &options).unwrap();
        let rejected = report.diagnostics.iter().any(|item| {
            matches!(
                item.code.as_str(),
                "datapack.type-mismatch" | "datapack.enum-value"
            )
        });
        assert_eq!(
            rejected, !valid,
            "field={field}, value={value}, diagnostics={:?}",
            report.diagnostics
        );
    }

    fs::write(
        &selected,
        "type: cdo\npatches:\n  - target: /Script/Test.Integers\n    properties:\n      - path: mEnumMaskValues\n        op: append\n        value: High\n",
    )
    .unwrap();
    let report = lint_path(&selected, &options).unwrap();
    assert!(report
        .diagnostics
        .iter()
        .any(|item| item.code == "datapack.enum-value"));
}

#[test]
fn named_bitmask_operations_validate_exported_names_and_preserve_missing_metadata() {
    let (directory, _archive, index) = fixture_with_integer_fields(true);
    let selected = directory.path().join("bitmasks.cdo.yml");
    let options = configured_options(directory.path(), index, "");
    let cases = [
        ("mNamedFlags32", "A", true),
        ("mNamedFlags32", "'A | TestFlags::B'", true),
        ("mNamedFlags32", "[A, B]", true),
        ("mNamedFlags32", "'A|Missing'", false),
        ("mNamedFlags32", "[A, Missing]", false),
        ("mNamedFlags32", "'A_SUFFIX'", false),
        ("mNamedFlags32", "[]", false),
        ("mNamedFlags32", "'|'", false),
        ("mFlags32", "'A|B'", true),
        ("mFlags32", "[A, B]", true),
        ("mMaskValues32", "'Low|Sign'", true),
        ("mMaskValues32", "[Low, Sign]", true),
        ("mMaskValues32", "Negative", false),
        ("mMaskValues32", "EFlags_MAX", false),
        ("mBitIndices64", "'Low|Sign'", true),
        ("mBitIndices64", "Negative", false),
        ("mBitIndices64", "InvalidIndex", false),
        ("mBitIndices64", "EFlags_MAX", false),
        ("mNarrowFlags", "Low", true),
        ("mNarrowFlags", "OutOfWidth", false),
        ("mNarrowFlags", "[Low, OutOfWidth]", false),
        ("mExactMask64", "Large", true),
    ];
    for operation in ["set", "append", "prepend", "remove"] {
        for (field, value, valid) in cases {
            fs::write(&selected, format!(
                "type: cdo\npatches:\n  - target: /Script/Test.Integers\n    properties:\n      - path: {field}\n        op: {operation}\n        value: {value}\n"
            )).unwrap();
            let report = lint_path(&selected, &options).unwrap();
            let rejected = report.diagnostics.iter().any(|item| {
                matches!(
                    item.code.as_str(),
                    "datapack.type-mismatch" | "datapack.enum-value"
                )
            });
            assert_eq!(
                rejected, !valid,
                "operation={operation}, field={field}, value={value}, diagnostics={:?}",
                report.diagnostics
            );
        }
    }
}

#[test]
fn raw_bitmask_storage_range_does_not_extend_other_operations() {
    let (directory, _archive, index) = fixture_with_integer_fields(true);
    let selected = directory.path().join("bitmasks.cdo.yml");
    let options = configured_options(directory.path(), index, "");
    for operation in ["append", "prepend", "remove"] {
        for scalar in ["0", "3", "'3'", "2147483648", "'2147483648'"] {
            fs::write(&selected, format!(
                "type: cdo\npatches:\n  - target: /Script/Test.Integers\n    properties:\n      - path: mFlags32\n        op: {operation}\n        value: {scalar}\n"
            )).unwrap();
            let report = lint_path(&selected, &options).unwrap();
            assert!(
                report
                    .diagnostics
                    .iter()
                    .any(|item| item.code == "datapack.enum-value"),
                "operation={operation}, scalar={scalar}, diagnostics={:?}",
                report.diagnostics
            );
        }
    }
}

#[test]
fn real_reflected_fields_report_unknown_names_primitive_types_and_enum_values() {
    let (directory, _archive, index) = fixture();
    let selected = directory.path().join("example.cdo.yml");
    fs::write(&selected, "type: cdo\npatches:\n  - target: /Script/Test.Child\n    properties:\n      - path: mMissing\n        value: 1\n      - path: mCount\n        value: wrong\n      - path: mMode\n        value: RF_MISSING\n      - path: mFlag\n        value: true\n  - target: /Script/Test.Limited\n    properties:\n      - path: UnexportedField\n        value: 1\n").unwrap();
    let options = configured_options(directory.path(), index, "");
    let report = lint_path(&selected, &options).unwrap();
    let codes: Vec<_> = report
        .diagnostics
        .iter()
        .map(|item| item.code.as_str())
        .collect();
    for code in [
        "datapack.unknown-property",
        "datapack.type-mismatch",
        "datapack.enum-value",
    ] {
        assert!(
            codes.contains(&code),
            "missing {code}: {:?}",
            report.diagnostics
        );
    }
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|item| item.code == "datapack.unknown-property")
            .count(),
        1,
        "incomplete parent must not invent missing fields"
    );
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|item| item.yaml_path.contains("properties[3]")),
        "valid inherited field"
    );
}

#[test]
fn localized_text_maps_are_valid_ftext_values_only_with_a_key() {
    let (directory, _archive, index) = fixture();
    let selected = directory.path().join("example.cdo.yml");
    // property-types.mdx: `{ key, ns, source }` builds keyed FText; `key` is required.
    fs::write(&selected, "type: cdo\npatches:\n  - target: /Script/Test.Child\n    properties:\n      - path: mDisplayName\n        value: Plain text\n      - path: mDisplayName\n        value:\n          key: Camera_Name\n          ns: Example\n          source: Camera\n      - path: mDisplayName\n        value:\n          source: Camera\n      - path: mDisplayName\n        value:\n          - Camera\n").unwrap();
    let options = configured_options(directory.path(), index, "");
    let report = lint_path(&selected, &options).unwrap();
    let paths = report
        .diagnostics
        .iter()
        .filter(|item| item.code == "datapack.type-mismatch")
        .map(|item| item.yaml_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "$.patches[0].properties[2].value",
            "$.patches[0].properties[3].value"
        ],
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn reflected_text_accepts_table_identity_and_rejects_mixed_or_invalid_tables() {
    let (directory, _archive, index) = fixture();
    let selected = directory.path().join("example.cdo.yml");
    let options = configured_options(directory.path(), index, "");
    for (value, valid) in [
        ("{stringTable: /Example/ST_Text.ST_Text, key: Name}", true),
        ("null", true),
        ("{stringTable: '', key: Name}", false),
        ("{stringTable: [], key: Name}", false),
        ("{stringTable: /Example/ST_Text.ST_Text, key: ''}", false),
        (
            "{stringTable: /Example/ST_Text.ST_Text, key: Name, ns: Example}",
            false,
        ),
        (
            "{stringTable: /Example/ST_Text.ST_Text, key: Name, source: Example}",
            false,
        ),
    ] {
        fs::write(&selected, format!("type: cdo\npatches:\n  - target: /Script/Test.Child\n    properties:\n      - path: mDisplayName\n        value: {value}\n")).unwrap();
        let report = lint_path(&selected, &options).unwrap();
        let mismatches: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "datapack.type-mismatch")
            .collect();
        assert_eq!(mismatches.is_empty(), valid, "{value}: {mismatches:?}");
    }
}

#[test]
fn append_unique_rejects_scalar_properties_even_if_operand_matches_scalar_type() {
    let (directory, _archive, index) = fixture();
    let selected = directory.path().join("example.cdo.yml");
    fs::write(&selected, "type: cdo\npatches:\n  - target: /Script/Test.Child\n    properties:\n      - path: mCount\n        op: append_unique\n        value: 1\n").unwrap();
    let options = configured_options(directory.path(), index, "");
    let report = lint_path(&selected, &options).unwrap();
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d.code == "datapack.type-mismatch" && d.message.contains("array")));
}

#[test]
fn update_where_rejects_arrays_without_struct_elements() {
    let (directory, _archive, index) = fixture();
    let selected = directory.path().join("example.cdo.yml");
    fs::write(&selected, "type: cdo\npatches:\n  - target: /Script/Test.Child\n    properties:\n      - path: mCounts\n        op: update_where\n        where: {path: Value, equals: 1}\n        value: {Value: 2}\n").unwrap();
    let options = configured_options(directory.path(), index, "");
    let report = lint_path(&selected, &options).unwrap();
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "datapack.type-mismatch"
            && diagnostic.message.contains("array of structs")
    }));
}

#[test]
fn explicit_validation_flags_and_unknown_class_information_severity_are_effective() {
    let (directory, _archive, index) = fixture();
    let selected = directory.path().join("example.cdo.yml");
    fs::write(&selected, "type: cdo\npatches:\n  - target: /Script/Test.Missing\n    properties:\n      - path: mUnknown\n        value: 1\n  - target: /Script/Test.Child\n    properties:\n      - path: mCount\n        value: wrong\n").unwrap();
    let options = configured_options(directory.path(), index.clone(), "[datapack.validation]\ncheck_classes = true\ncheck_properties = false\nunknown_class_severity = 'information'\n");
    let report = lint_path(&selected, &options).unwrap();
    let unknown = report
        .diagnostics
        .iter()
        .find(|item| item.code == "datapack.unknown-class")
        .expect("missing target needs actual class diagnostic");
    assert_eq!(
        serde_json::to_value(unknown).unwrap()["severity"],
        "information"
    );
    assert_eq!((report.errors, report.warnings), (0, 0));
    assert!(report.ok);
    assert!(!report
        .diagnostics
        .iter()
        .any(|item| item.code == "datapack.type-mismatch"));
    let options = configured_options(directory.path(), index, "[datapack.validation]\ncheck_classes = false\ncheck_properties = false\nunknown_class_severity = 'error'\n");
    assert!(!lint_path(&selected, &options)
        .unwrap()
        .diagnostics
        .iter()
        .any(|item| item.code.starts_with("datapack.")));
}

#[test]
fn scoped_validation_preferences_preserve_false_and_inherit_invalid_project_values() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user.toml");
    fs::write(&user, "[datapack.validation]\ncheck_classes = false\ncheck_properties = false\ninlay_hints = false\nunknown_class_severity = 'information'\n").unwrap();
    fs::write(directory.path().join(".kdf-editor.toml"), "[datapack.validation]\ncheck_classes = true\ncheck_properties = 'invalid'\ninlay_hints = true\nunknown_class_severity = 'invalid'\n").unwrap();
    let options = LintSettings::load(directory.path(), Some(&user))
        .unwrap()
        .lint_options(Profile::Runtime);
    assert!(options.datapack_validation.check_classes);
    assert!(!options.datapack_validation.check_properties);
    assert!(options.datapack_validation.inlay_hints);
    assert_eq!(
        options.datapack_validation.unknown_class_severity,
        kdf_validation::UnknownClassSeverity::Information
    );
}

#[test]
fn substituted_config_operands_avoid_literal_type_checks_but_quoted_refs_still_are_literals() {
    let (directory, _archive, index) = fixture();
    fs::write(directory.path().join("balance.config.yml"), "category: /Script/FactoryGame.FGCategory\nfields:\n  - id: amount\n    type: number\n    name: Amount\n    description: Count\n    default: 2\n").unwrap();
    let selected = directory.path().join("example.cdo.yml");
    fs::write(&selected, "type: cdo\npatches:\n  - target: /Script/Test.Child\n    properties:\n      - path: mCount\n        value: cfg:amount\n      - path: mCount\n        value: 'cfg:amount'\n      - path: mCount\n        value: '{cfg:amount}'\n      - path: mCount\n        value: wrong\n").unwrap();
    let options = configured_options(directory.path(), index, "");
    let report = lint_path(&selected, &options).unwrap();
    let paths = report
        .diagnostics
        .iter()
        .filter(|item| item.code == "datapack.type-mismatch")
        .map(|item| item.yaml_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "$.patches[0].properties[1].value",
            "$.patches[0].properties[3].value"
        ],
        "{:?}",
        report.diagnostics
    );
}
