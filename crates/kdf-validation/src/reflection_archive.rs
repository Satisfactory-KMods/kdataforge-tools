use std::fs::File;

use zip::ZipArchive;

pub(crate) fn unique_reflection_prefix(
    archive: &ZipArchive<File>,
) -> Result<Option<&'static str>, &'static str> {
    let mut selected = None;
    for prefix in ["reflection/", "", "kdf/reflection/"] {
        if archive
            .file_names()
            .any(|name| name == format!("{prefix}manifest.json"))
        {
            if selected.is_some() {
                return Err("datapack has multiple KDF reflection manifests");
            }
            selected = Some(prefix);
        }
    }
    Ok(selected)
}
