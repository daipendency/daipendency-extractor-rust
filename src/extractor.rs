use super::{api, dependencies, metadata};
use daipendency_extractor::{
    DependencyResolutionError, ExtractionError, Extractor, LibraryMetadata, LibraryMetadataError,
    Namespace,
};
use std::path::Path;
use tree_sitter::{Language, Parser};

pub struct RustExtractor;

impl Default for RustExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl RustExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Extractor for RustExtractor {
    fn get_parser_language(&self) -> Language {
        tree_sitter_rust::LANGUAGE.into()
    }

    fn get_library_metadata(&self, path: &Path) -> Result<LibraryMetadata, LibraryMetadataError> {
        metadata::extract_metadata(path)
    }

    fn extract_public_api(
        &self,
        metadata: &LibraryMetadata,
        parser: &mut Parser,
    ) -> Result<Vec<Namespace>, ExtractionError> {
        api::build_public_api(&metadata.entry_point, &metadata.name, parser)
    }

    fn resolve_dependency_path(
        &self,
        dependency_name: &str,
        dependant_path: &Path,
    ) -> Result<std::path::PathBuf, DependencyResolutionError> {
        dependencies::resolve_dependency_path(dependency_name, dependant_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::setup_parser;
    use assertables::{assert_contains, assert_ok};
    use daipendency_testing::tempdir::TempDir;

    #[test]
    fn get_package_metadata() {
        let temp_dir = TempDir::new();
        let cargo_toml = temp_dir
            .create_file(
                "Cargo.toml",
                r#"[package]
name = "test_crate"
version = "0.1.0"
"#,
            )
            .unwrap();

        let analyser = RustExtractor::new();
        let metadata = analyser
            .get_library_metadata(cargo_toml.parent().unwrap())
            .unwrap();

        assert_eq!(metadata.name, "test_crate");
    }

    #[test]
    fn extract_public_api() {
        let temp_dir = TempDir::new();
        let lib_rs = temp_dir
            .create_file(
                "src/lib.rs",
                r#"
pub fn test_function() -> i32 {
    42
}
"#,
            )
            .unwrap();

        let analyser = RustExtractor::new();
        let metadata = LibraryMetadata {
            name: "test_crate".to_string(),
            version: Some("0.1.0".to_string()),
            documentation: String::new(),
            entry_point: lib_rs,
        };
        let mut parser = setup_parser();

        let namespaces = analyser.extract_public_api(&metadata, &mut parser).unwrap();

        assert_eq!(namespaces.len(), 1);
        let root = namespaces.iter().find(|n| n.name == "test_crate").unwrap();
        assert_eq!(root.symbols.len(), 1);
        assert_eq!(root.symbols[0].name, "test_function");
    }

    #[test]
    fn resolve_dependency_path_success() {
        let cargo_toml = Path::new(env!("CARGO_MANIFEST_DIR"));
        let analyser = RustExtractor::new();
        let dependency_name = "tree-sitter";

        let result = analyser.resolve_dependency_path(dependency_name, &cargo_toml);

        assert_ok!(&result);
        assert_contains!(result.unwrap().to_str().unwrap(), dependency_name);
    }
}
