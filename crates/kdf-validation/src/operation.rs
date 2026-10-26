//! Single operation vocabulary shared by validation, completion, signature help and Visual mode.

pub const TAG_CONTAINER_SELECTOR: &str = "@tagContainers";
pub const TAG_CONTAINER_SELECTOR_DOC: &str = "Select every top-level FGameplayTagContainer on each actual target, including inherited fields and fields on late-loaded subclasses. Quote this YAML path. Supports set, append, prepend, remove and clear. Nested containers are not selected; no matching fields is a no-op. Operation-only: not valid in where, from or template reads.";

pub fn tag_container_operation(name: &str) -> bool {
    matches!(name, "set" | "append" | "prepend" | "remove" | "clear")
}

/// Operand metadata for an operation selector, not a concrete readable property path.
pub fn tag_container_selector_property(path: &str) -> Option<crate::PropertyInfo> {
    (path == TAG_CONTAINER_SELECTOR).then(|| {
        crate::PropertyInfo::from_record(&serde_json::json!({
            "name": TAG_CONTAINER_SELECTOR,
            "kind": "StructProperty",
            "cppType": "FGameplayTagContainer",
            "struct": "/Script/GameplayTags.GameplayTagContainer"
        }))
    })
}

#[derive(Clone, Copy, Debug)]
pub struct OperationSpec {
    pub name: &'static str,
    pub required: &'static [&'static str],
    pub description: &'static str,
}

pub const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "add",
        required: &["value"],
        description: "Add value to a numeric property.",
    },
    OperationSpec {
        name: "append",
        required: &["value"],
        description: "Append value to an array.",
    },
    OperationSpec {
        name: "append_unique",
        required: &["value"],
        description:
            "Append only missing array elements, preserving existing duplicates and order.",
    },
    OperationSpec {
        name: "clamp",
        required: &["min", "max"],
        description: "Limit a numeric property to min and max.",
    },
    OperationSpec {
        name: "clear",
        required: &[],
        description: "Clear a container or value.",
    },
    OperationSpec {
        name: "copy",
        required: &["from"],
        description: "Copy another property into this one.",
    },
    OperationSpec {
        name: "divide",
        required: &["value"],
        description: "Divide a numeric property by value.",
    },
    OperationSpec {
        name: "duplicate",
        required: &["index"],
        description: "Duplicate an array element.",
    },
    OperationSpec {
        name: "insert",
        required: &["index", "value"],
        description: "Insert value at an array index.",
    },
    OperationSpec {
        name: "max",
        required: &["value"],
        description: "Keep the larger of property and value.",
    },
    OperationSpec {
        name: "min",
        required: &["value"],
        description: "Keep the smaller of property and value.",
    },
    OperationSpec {
        name: "move",
        required: &["from"],
        description: "Move another property into this one.",
    },
    OperationSpec {
        name: "multiply",
        required: &["value"],
        description: "Multiply a numeric property by value.",
    },
    OperationSpec {
        name: "prepend",
        required: &["value"],
        description: "Prepend value to an array.",
    },
    OperationSpec {
        name: "remove",
        required: &["value"],
        description: "Remove matching value from an array.",
    },
    OperationSpec {
        name: "remove_at",
        required: &["index"],
        description: "Remove an array element at index.",
    },
    OperationSpec {
        name: "replace_refs",
        required: &["from", "value"],
        description: "Recursively replace matching object and class references below a property.",
    },
    OperationSpec {
        name: "replace",
        required: &["index", "value"],
        description: "Replace an array element at index.",
    },
    OperationSpec {
        name: "reverse",
        required: &[],
        description: "Reverse array order.",
    },
    OperationSpec {
        name: "set",
        required: &["value"],
        description: "Set the property to value.",
    },
    OperationSpec {
        name: "sort",
        required: &[],
        description: "Sort an array.",
    },
    OperationSpec {
        name: "subtract",
        required: &["value"],
        description: "Subtract value from a numeric property.",
    },
    OperationSpec {
        name: "swap",
        required: &["index", "with"],
        description: "Swap two array elements.",
    },
    OperationSpec {
        name: "unique",
        required: &[],
        description: "Remove duplicate array elements.",
    },
    OperationSpec {
        name: "update_where",
        required: &["where", "value"],
        description: "Partially update every matching struct element in an array.",
    },
];

pub fn operation_spec(name: &str) -> Option<&'static OperationSpec> {
    OPERATIONS.iter().find(|spec| spec.name == name)
}
