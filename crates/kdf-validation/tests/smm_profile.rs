use kdf_validation::{compare_required_mods, parse_profiles_json, SmmModState};

fn refs(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn selected_profile_reports_enabled_disabled_and_missing_mods() {
    let document = parse_profiles_json(
        r#"{
          "profiles": {
            "Default": {"name":"Default","required_targets":[],"mods":{}},
            "Modded": {"name":"Modded","required_targets":["windows"],"mods":{
              "SML":{"version":"3.11.1","enabled":true},
              "SatisfactoryPlus":{"version":"1.0.0","enabled":false}
            }}
          },
          "selected_profile":"Modded",
          "version":0
        }"#,
    )
    .unwrap();
    assert_eq!(document.version, Some(0));
    assert_eq!(document.profiles.as_ref().unwrap().len(), 2);
    assert_eq!(
        document.profiles.as_ref().unwrap()["Modded"]
            .required_targets
            .as_ref()
            .unwrap(),
        &["windows"]
    );
    let comparison = compare_required_mods(
        &document,
        &refs(&["SML", "SatisfactoryPlus", "NotInstalled"]),
    );
    assert_eq!(comparison.selected_profile.as_deref(), Some("Modded"));
    assert_eq!(comparison.mods[0].state, SmmModState::Enabled);
    assert_eq!(comparison.mods[0].version.as_deref(), Some("3.11.1"));
    assert_eq!(comparison.mods[1].state, SmmModState::Disabled);
    assert_eq!(comparison.mods[2].state, SmmModState::Missing);
}

#[test]
fn absent_selection_never_falls_back_to_an_arbitrary_profile() {
    let document = parse_profiles_json(
        r#"{"profiles":{"Only":{"mods":{"SML":{"enabled":true}}}},"version":0}"#,
    )
    .unwrap();
    let comparison = compare_required_mods(&document, &refs(&["SML"]));
    assert_eq!(comparison.selected_profile, None);
    assert_eq!(comparison.mods[0].state, SmmModState::Unknown);
}

#[test]
fn malformed_or_missing_fields_remain_unknown_without_claiming_absence() {
    let missing_mods = parse_profiles_json(
        r#"{"profiles":{"Modded":{"name":"Modded","mods":null}},"selected_profile":"Modded"}"#,
    )
    .unwrap();
    assert_eq!(
        compare_required_mods(&missing_mods, &refs(&["SML"])).mods[0].state,
        SmmModState::Unknown
    );

    let partial = parse_profiles_json(
        r#"{"profiles":{"Modded":{"mods":{"SML":{"version":"3.11.1"},"Bad":"broken"}}},"selected_profile":"Modded"}"#,
    )
    .unwrap();
    let comparison = compare_required_mods(&partial, &refs(&["SML", "Bad", "Absent"]));
    assert_eq!(comparison.mods[0].state, SmmModState::Unknown);
    assert_eq!(comparison.mods[1].state, SmmModState::Unknown);
    assert_eq!(comparison.mods[2].state, SmmModState::Missing);
}

#[test]
fn missing_selected_profile_object_and_profiles_map_are_unknown() {
    for json in [
        r#"{"profiles":{},"selected_profile":"Deleted"}"#,
        r#"{"profiles":[],"selected_profile":"Deleted"}"#,
    ] {
        let document = parse_profiles_json(json).unwrap();
        let comparison = compare_required_mods(&document, &refs(&["SML"]));
        assert_eq!(comparison.mods[0].state, SmmModState::Unknown);
        assert!(!comparison.issues.is_empty());
    }
}

#[test]
fn exact_references_are_deduplicated_and_missing_version_does_not_hide_enabled_state() {
    let document = parse_profiles_json(
        r#"{"profiles":{"Selected":{"mods":{"SML":{"enabled":true}}}},"selected_profile":"Selected","extra":"ignored"}"#,
    )
    .unwrap();
    let comparison = compare_required_mods(&document, &refs(&["SML", "SML", "sml"]));
    assert_eq!(comparison.mods.len(), 2);
    assert_eq!(comparison.mods[0].state, SmmModState::Enabled);
    assert_eq!(comparison.mods[0].version, None);
    assert_eq!(comparison.mods[1].state, SmmModState::Missing);
}

#[test]
fn invalid_or_oversized_json_is_rejected() {
    assert!(parse_profiles_json("{").is_err());
    assert!(parse_profiles_json("[]").is_err());
    assert!(parse_profiles_json(&"x".repeat(4 * 1024 * 1024 + 1)).is_err());
}
