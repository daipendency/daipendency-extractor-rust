mod module_directory;
mod module_extraction;
mod namespace_construction;
mod parsing;
mod symbol_collection;
mod symbol_resolution;
mod test_helpers;

use daipendency_extractor::ExtractionError;
use daipendency_extractor::Namespace;
use module_extraction::extract_modules;
use std::path::Path;
use tree_sitter::Parser;

use namespace_construction::construct_namespaces;
use symbol_collection::collect_module_directories;
use symbol_resolution::resolve_symbols;

pub fn build_public_api(
    entry_point: &Path,
    crate_name: &str,
    parser: &mut Parser,
) -> Result<Vec<Namespace>, ExtractionError> {
    let module_directories = collect_module_directories(entry_point, parser)?;
    let modules = extract_modules(&module_directories)?;
    let resolution = resolve_symbols(&modules)?;
    let namespaces = construct_namespaces(resolution, crate_name);
    Ok(namespaces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::setup_parser;
    use assertables::assert_matches;
    use daipendency_testing::tempdir::TempDir;

    const STUB_CRATE_NAME: &str = "test_crate";

    #[test]
    fn nonexistent_file() {
        let mut parser = setup_parser();
        let path = std::path::PathBuf::from("nonexistent.rs");

        let result = build_public_api(&path, STUB_CRATE_NAME, &mut parser);

        assert_matches!(result, Err(ExtractionError::Io(_)));
    }

    #[test]
    fn integration() {
        let temp_dir = TempDir::new();
        let lib_rs = temp_dir
            .create_file(
                "src/lib.rs",
                r#"
pub mod module;
pub use module::Format;

pub fn process(format: Format) -> String {
    "processed".to_string()
}
"#,
            )
            .unwrap();
        temp_dir
            .create_file(
                "src/module.rs",
                r#"
pub enum Format {
    Text,
    Binary,
}
"#,
            )
            .unwrap();
        let mut parser = setup_parser();

        let namespaces = build_public_api(&lib_rs, STUB_CRATE_NAME, &mut parser).unwrap();

        assert_eq!(namespaces.len(), 2);
        let root = namespaces
            .iter()
            .find(|n| n.name == STUB_CRATE_NAME)
            .unwrap();
        assert_eq!(root.symbols.len(), 2);
        assert!(root.symbols.iter().any(|s| s.name == "process"));
        assert!(root.symbols.iter().any(|s| s.name == "Format"));

        let module = namespaces
            .iter()
            .find(|n| n.name == format!("{}::module", STUB_CRATE_NAME))
            .unwrap();
        assert_eq!(module.symbols.len(), 1);
        assert!(module.symbols.iter().any(|s| s.name == "Format"));
    }

    #[test]
    fn wildcard_reexport() {
        let temp_dir = TempDir::new();
        let lib_rs = temp_dir
            .create_file(
                "src/lib.rs",
                r#"
mod submodule;
pub use submodule::*;
"#,
            )
            .unwrap();
        temp_dir
            .create_file(
                "src/submodule.rs",
                r#"
pub struct One;
pub struct Two;
"#,
            )
            .unwrap();
        let mut parser = setup_parser();

        let namespaces = build_public_api(&lib_rs, STUB_CRATE_NAME, &mut parser).unwrap();

        assert_eq!(namespaces.len(), 1);
        let root = &namespaces[0];
        assert_eq!(root.symbols.len(), 2);
        assert!(root.get_symbol("One").is_some());
        assert!(root.get_symbol("Two").is_some());
    }

    #[test]
    fn new_style_module_directory() {
        let temp_dir = TempDir::new();
        let lib_rs = temp_dir
            .create_file("src/lib.rs", r#"pub mod module;"#)
            .unwrap();
        temp_dir
            .create_file("src/module.rs", r#"pub mod submodule;"#)
            .unwrap();
        temp_dir
            .create_file("src/module/submodule.rs", r#"pub struct Foo;"#)
            .unwrap();
        let mut parser = setup_parser();

        let namespaces = build_public_api(&lib_rs, STUB_CRATE_NAME, &mut parser).unwrap();

        assert_eq!(namespaces.len(), 1);
        let namespace = &namespaces[0];
        assert_eq!(
            namespace.name,
            format!("{STUB_CRATE_NAME}::module::submodule")
        );
        assert_eq!(namespace.symbols.len(), 1);
        assert!(namespace.get_symbol("Foo").is_some());
    }

    #[test]
    fn external_dependency_reexport() {
        let temp_dir = TempDir::new();
        let lib_rs = temp_dir
            .create_file(
                "src/lib.rs",
                r#"
pub use serde_json;
"#,
            )
            .unwrap();
        let mut parser = setup_parser();

        let namespaces = build_public_api(&lib_rs, STUB_CRATE_NAME, &mut parser).unwrap();

        assert_eq!(namespaces.len(), 1);
        let root = &namespaces[0];
        assert_eq!(root.name, STUB_CRATE_NAME);
        assert_eq!(root.symbols.len(), 1);
        let symbol = root.get_symbol("serde_json").unwrap();
        assert_eq!(symbol.source_code, "pub use serde_json;");
    }
}
