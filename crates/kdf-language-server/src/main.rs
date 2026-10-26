mod actions;
mod authoring;
mod cards;
mod class_slots;
mod completion;
#[cfg(test)]
mod completion_tests;
mod config_nav;
mod context;
mod docs;
#[cfg(test)]
mod feature_tests;
mod features;
mod generated;
mod hover;
mod intellisense;
mod navigation;
mod pack_scope;
mod protocol;
mod rename;
mod resolve;
mod schema_hover;
mod selector_context;
mod semantic;
mod signature;
mod state;
#[cfg(test)]
mod test_fixture;
#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;
mod workspace;

use state::Backend;
use tower_lsp::{LspService, Server};

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let (service, socket) = LspService::build(Backend::new)
            .custom_method("kdf/contentCompletion", Backend::content_completion)
            .custom_method("kdf/contentSymbols", Backend::content_symbols)
            .custom_method("kdf/renameContent", Backend::rename_content)
            .custom_method("kdf/effectiveSchema", Backend::effective_schema)
            .custom_method("kdf/explain", Backend::explain)
            .custom_method("kdf/why", Backend::explain)
            .custom_method("kdf/exportSnapshot", Backend::export_snapshot)
            .custom_method("kdf/classes", Backend::classes)
            .custom_method("kdf/classInfo", Backend::class_info)
            .custom_method("kdf/classReferenceSlots", Backend::class_reference_slots)
            .custom_method("kdf/affectedElements", Backend::affected_elements)
            .custom_method("kdf/inferPropertyOwner", Backend::infer_property_owner)
            .custom_method("kdf/operations", Backend::operations)
            .custom_method("kdf/previewBinding", Backend::preview_binding)
            .custom_method("kdf/revalidate", Backend::revalidate)
            .custom_method("kdf/reindex", Backend::reindex)
            .finish();
        Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
            .serve(service)
            .await;
    });
    // `exit` was received. Do not wait for what still runs on the blocking pool (the read of stdin
    // that the client keeps open, an index warm-up, a lint): the process ends now.
    runtime.shutdown_background();
}
