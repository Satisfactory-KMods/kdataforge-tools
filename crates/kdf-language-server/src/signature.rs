//! Signature help for template function calls (`{item:Desc_X_C:Fn(a, |b)}`) and `op:` values.

use crate::context::{utf16_len, Place, Resolved};
use crate::semantic::{template_target, Env};
use kdf_validation::template::{self, Stage};
use kdf_validation::{display_type, explicit_params, operation_spec, Callable, Implicit};
use tower_lsp::lsp_types::{
    Documentation, MarkupContent, MarkupKind, ParameterInformation, ParameterLabel, SignatureHelp,
    SignatureInformation,
};

fn documentation(env: &Env<'_>, text: String) -> Option<Documentation> {
    if text.is_empty() {
        return None;
    }
    Some(if env.caps.signature_markdown {
        Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: text,
        })
    } else {
        Documentation::String(crate::resolve::plain_text(&text))
    })
}

pub(crate) fn signature_help(env: &Env<'_>, resolved: &Resolved) -> Option<SignatureHelp> {
    let Place::Value(slot) = &resolved.place else {
        return None;
    };
    let scalar = &slot.scalar;
    if scalar.value.contains('{') {
        if let Some(help) = call_help(env, scalar) {
            return Some(help);
        }
    }
    // `op: multiply` shows what the operation needs.
    if resolved.key() == Some("op") {
        let name = scalar.value.trim();
        let spec = operation_spec(name)?;
        let label = if spec.required.is_empty() {
            format!("{name}()")
        } else {
            format!("{name}({})", spec.required.join(", "))
        };
        return Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label,
                documentation: documentation(env, spec.description.to_owned()),
                parameters: None,
                active_parameter: None,
            }],
            active_signature: Some(0),
            active_parameter: None,
        });
    }
    None
}

fn call_help(env: &Env<'_>, scalar: &crate::context::Scalar) -> Option<SignatureHelp> {
    let partial = template::partial_at(&scalar.value, scalar.cursor)?;
    if partial.stage != Stage::Argument {
        return None;
    }
    let call = partial.function.as_ref()?;
    let target = template_target(env, &partial.prefix, partial.target.as_deref()?)?;
    let function = env.reflection.find_function(&target.class, &call.name)?;
    let callable = env
        .reflection
        .callable(&function, target.kind, Some(&target.class));
    let implicit = match &callable {
        Callable::Yes { implicit, .. } => *implicit,
        Callable::No(_) => Implicit::None,
    };
    let params = explicit_params(&function, implicit);
    // Label `Name(a: T, b: U) -> R` with the offset of each parameter inside it.
    let mut label = format!("{}(", function.display_name());
    let mut parameters = Vec::new();
    for (index, param) in params.iter().enumerate() {
        if index > 0 {
            label.push_str(", ");
        }
        let text = format!(
            "{}: {}",
            param.property.display_name(),
            display_type(&param.property)
        );
        let start = utf16_len(&label);
        label.push_str(&text);
        let end = utf16_len(&label);
        parameters.push(ParameterInformation {
            // Offsets only for clients that declared `labelOffsetSupport`; the others look the
            // parameter text up inside the signature label.
            label: if env.caps.label_offsets {
                ParameterLabel::LabelOffsets([start, end])
            } else {
                ParameterLabel::Simple(text)
            },
            documentation: None,
        });
    }
    label.push(')');
    if let Some(ret) = &function.ret {
        label.push_str(&format!(" -> {}", display_type(ret)));
    }
    let mut notes = Vec::new();
    match implicit {
        Implicit::Class => notes.push("The target class is passed implicitly.".to_owned()),
        Implicit::Object => notes.push("The target object is passed implicitly.".to_owned()),
        Implicit::None => {}
    }
    if let Callable::No(reason) = &callable {
        notes.push(format!("Not callable in templates: {reason}"));
    }
    let active = if params.is_empty() {
        None
    } else {
        Some(call.arg_index.min(params.len() as u32 - 1))
    };
    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label,
            documentation: documentation(env, notes.join("\n\n")),
            parameters: Some(parameters),
            active_parameter: env
                .caps
                .signature_active_parameter
                .then_some(active)
                .flatten(),
        }],
        active_signature: Some(0),
        active_parameter: active,
    })
}
