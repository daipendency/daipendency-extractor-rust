use super::{api, metadata};
use daipendency_extractor::{
    ExtractionError, Extractor, LibraryMetadata, LibraryMetadataError, Namespace,
};
use std::path::Path;
use tree_sitter::{Language, Parser};

pub struct RustExtractor;

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{create_temp_dir, setup_parser};

    #[test]
    fn get_package_metadata() {
        let temp_dir = create_temp_dir();
        let cargo_toml = temp_dir.path().join("Cargo.toml");
        std::fs::write(
            &cargo_toml,
            r#"[package]
name = "test_crate"
version = "0.1.0"
"#,
        )
        .unwrap();

        let analyser = RustExtractor::new();
        let metadata = analyser.get_library_metadata(temp_dir.path()).unwrap();

        assert_eq!(metadata.name, "test_crate");
    }

    #[test]
    fn extract_public_api() {
        let temp_dir = create_temp_dir();
        let src_dir = temp_dir.path().join("src");
        std::fs::create_dir(&src_dir).unwrap();
        let lib_rs = src_dir.join("lib.rs");
        std::fs::write(
            &lib_rs,
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
}
