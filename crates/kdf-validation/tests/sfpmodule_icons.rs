use kdf_validation::{validate_document, LintOptions};
use std::path::Path;

fn icon_diagnostics(icon: Option<&str>) -> Vec<kdf_validation::Diagnostic> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mod-schema/DataForge/Example/icon.sfpmodule.yml");
    let mut source = String::from(
        "type: sfpmodule\nid: WaterDam\nname:\n  key: WaterDam.Name\ndescription:\n  key: WaterDam.Description\n",
    );
    if let Some(icon) = icon {
        source.push_str(&format!("icon: {}\n", serde_json::to_string(icon).unwrap()));
    }
    validate_document(&path, &source, &LintOptions::default())
        .unwrap()
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.code.starts_with("schema."))
        .collect()
}

#[test]
fn accepts_empty_relative_and_unmounted_texture_paths() {
    for icon in [
        None,
        Some(""),
        Some("assets/_water-dam.v2.png"),
        Some("/RefinedPower/Path/Icon_WaterDam.Icon_WaterDam"),
        Some("/NotInstalled/Textures/Icon.Icon"),
        Some("/Unknown/Größe+1.Icon-1"),
    ] {
        let diagnostics = icon_diagnostics(icon);
        assert!(diagnostics.is_empty(), "{icon:?}: {diagnostics:?}");
    }
}

#[test]
fn object_path_length_limit_does_not_limit_relative_paths() {
    let maximum_object = format!("/Root/{}.Icon", "a".repeat(1013));
    assert_eq!(maximum_object.len(), 1024);
    assert!(icon_diagnostics(Some(&maximum_object)).is_empty());
    assert!(icon_diagnostics(Some(&(maximum_object + "x")))
        .iter()
        .any(|diagnostic| diagnostic.code == "schema.maxLength"));
    assert!(icon_diagnostics(Some(&format!("assets/{}.png", "a".repeat(2048)))).is_empty());
}

#[test]
fn malformed_icons_report_the_icon_location() {
    for icon in [
        "../icon.png",
        "assets//icon.png",
        "assets/.icon.png",
        "assets/icon name.png",
        "assets/icon\\name.png",
        "/RootOnly.Icon",
        "/Root//Icon.Icon",
        "/Root/Icon.",
        "/Root/.Icon",
        "/Root/../Icon.Icon",
        "/Root/Icon.Icon.More",
        "/Root/Icon.Icon:Subobject",
        "/Root/Icon\\Name.Icon",
        "/Root/Bad?Package.Icon",
        "/Root/Bad Package.Icon",
        "/Root/Icon.Bad@Object",
        "/Root/Icon.None",
        "/Root/Icon.nOnE",
        "/Root/Icon.Icon\n",
    ] {
        let diagnostics = icon_diagnostics(Some(icon));
        assert!(!diagnostics.is_empty(), "accepted {icon:?}");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.yaml_path == "$.icon"),
            "{icon:?}: {diagnostics:?}"
        );
    }
}
