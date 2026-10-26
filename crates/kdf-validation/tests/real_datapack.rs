//! Smoke test against a real reflection export. Skipped unless `KDF_TEST_DATAPACK` points to a
//! `.kdfdata` archive: `KDF_TEST_DATAPACK=<file> cargo test -p kdf-validation --test real_datapack -- --nocapture`.
use kdf_validation::{PropertyKind, ReflectionIndex, ReflectionStatus, Reload, TargetKind};
use std::path::PathBuf;
use std::time::Instant;

#[test]
fn real_datapack_queries_work_and_stay_fast() {
    let Some(path) = std::env::var_os("KDF_TEST_DATAPACK").map(PathBuf::from) else {
        eprintln!("KDF_TEST_DATAPACK not set; skipping");
        return;
    };
    let started = Instant::now();
    let index = ReflectionIndex::load(&path);
    eprintln!(
        "load: {:?}, {} classes",
        started.elapsed(),
        index.classes.len()
    );
    assert!(matches!(
        index.status,
        ReflectionStatus::Ready | ReflectionStatus::Limited
    ));

    let started = Instant::now();
    let recipe = "/Script/FactoryGame.FGRecipe";
    assert_eq!(index.class_path("fgrecipe"), Some(recipe));
    let duration = index
        .find_property(recipe, "mManufactoringDuration")
        .expect("recipe duration");
    assert_eq!(duration.kind, PropertyKind::Float);
    eprintln!("first lookups (lazy maps): {:?}", started.elapsed());

    let started = Instant::now();
    if let Some(item_class) = index.resolve_property_path(recipe, "mIngredients[0].ItemClass") {
        assert_eq!(item_class.kind, PropertyKind::Class);
    }
    let items = index.descendants("/Script/FactoryGame.FGItemDescriptor", true);
    assert!(!items.is_empty());
    let members = index.list_members("/Script/FactoryGame.FGItemDescriptor", TargetKind::Class);
    assert!(members.iter().any(|member| member.name == "mStackSize"));
    eprintln!(
        "struct index + descendants + members: {:?} ({} descendants, {} members)",
        started.elapsed(),
        items.len(),
        members.len()
    );

    let (reload, _) = index.property_reload("/Script/FactoryGame.FGBuildable", "mHealth");
    assert!(matches!(
        reload,
        Reload::NewActors | Reload::Hot | Reload::Cold
    ));
    let kinds = index
        .classes
        .iter()
        .take(3000)
        .filter_map(|class| index.class_info(&class.path))
        .flat_map(|record| record["properties"].as_array().cloned().unwrap_or_default())
        .map(|property| kdf_validation::PropertyInfo::from_record(&property).kind)
        .filter(|kind| *kind == PropertyKind::Other)
        .count();
    eprintln!("properties with unrecognised kind among the first 3000 classes: {kinds}");
    assert_eq!(
        kinds, 0,
        "every exported property kind should be recognised"
    );
}
