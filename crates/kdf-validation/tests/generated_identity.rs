use kdf_validation::{generated_identity, GeneratedNamespace};

#[test]
fn class_ids_keep_runtime_case_and_punctuation_normalization() {
    assert_eq!(
        generated_identity("item", "Desc-A.B"),
        Some((GeneratedNamespace::Class, "desc_a_b".into()))
    );
    assert_eq!(
        generated_identity("recipe", "DESC_A_B"),
        generated_identity("item", "Desc-A.B")
    );
}

#[test]
fn assets_curves_sounds_and_dataassets_share_one_namespace() {
    for kind in ["asset", "curve", "sound", "dataasset"] {
        assert_eq!(
            generated_identity(kind, "Icon-Test"),
            Some((GeneratedNamespace::Asset, "icon_test".into()))
        );
    }
    assert_ne!(
        generated_identity("asset", "Icon-Test"),
        generated_identity("item", "Icon-Test")
    );
}

#[test]
fn eventrules_keep_punctuation_but_fold_case() {
    assert_eq!(
        generated_identity("eventrule", "Event-Test"),
        Some((GeneratedNamespace::EventRule, "event-test".into()))
    );
    assert_ne!(
        generated_identity("eventrule", "Event-Test"),
        generated_identity("eventrule", "Event_Test")
    );
}

#[test]
fn resource_placements_keep_case_and_punctuation() {
    assert_eq!(
        generated_identity("resourcenode", "Node-Test"),
        Some((GeneratedNamespace::ResourcePlacement, "Node-Test".into()))
    );
    assert_ne!(
        generated_identity("resourcenode", "Node-Test"),
        generated_identity("resourcenode", "node-test")
    );
}

#[test]
fn config_field_ids_do_not_claim_generated_identity() {
    assert_eq!(generated_identity("config", "Desc_Test"), None);
}
