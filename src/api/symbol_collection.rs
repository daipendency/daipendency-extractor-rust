use daipendency_extractor::ExtractionError;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tree_sitter::Parser;

use super::module_directory::ModuleDirectory;
use super::parsing::{parse_rust_file, RustSymbol};

enum LocalModuleType {
    File,
    Directory(String),
}

struct LocalModuleImport {
    path: String,
    module_type: LocalModuleType,
}

/// Traverse the source files of the Rust crate and collect all symbols and symbol references (reexports).
pub fn collect_module_directories(
    entry_point: &Path,
    parser: &mut Parser,
) -> Result<Vec<ModuleDirectory>, ExtractionError> {
    recursively_collect_module_directories(
        entry_point,
        entry_point.parent().unwrap(),
        true,
        "",
        parser,
    )
}

fn recursively_collect_module_directories(
    entry_point_path: &Path,
    directory_path: &Path,
    is_root_directory_public: bool,
    namespace_prefix: &str,
    parser: &mut Parser,
) -> Result<Vec<ModuleDirectory>, ExtractionError> {
    let entry_point_content =
        std::fs::read_to_string(entry_point_path).map_err(ExtractionError::Io)?;

    let entry_point_file = parse_rust_file(&entry_point_content, parser)?;

    let mut internal_files = HashMap::new();
    let mut imported_directories = Vec::new();
    for symbol in &entry_point_file.symbols {
        if let RustSymbol::ModuleImport {
            name,
            is_reexported,
        } = symbol
        {
            let import = categorise_module_import(entry_point_path, directory_path, name)?;
            match import.module_type {
                LocalModuleType::File => {
                    let file = parse_rust_file(&std::fs::read_to_string(&import.path)?, parser)?;
                    internal_files.insert(name.clone(), file);
                }
                LocalModuleType::Directory(ref module_dir) => {
                    let module_name = prefix_namespace(name, namespace_prefix);
                    let directories = recursively_collect_module_directories(
                        &PathBuf::from(&import.path),
                        &PathBuf::from(module_dir),
                        *is_reexported,
                        &module_name,
                        parser,
                    )?;
                    imported_directories.extend(directories);
                }
            }
        }
    }

    let root_module_directory = ModuleDirectory {
        name: namespace_prefix.to_string(),
        is_public: is_root_directory_public,
        entry_point: entry_point_file,
        internal_files,
    };
    let mut directories = vec![root_module_directory];
    directories.extend(imported_directories);
    Ok(directories)
}

fn categorise_module_import(
    current_file: &Path,
    directory_path: &Path,
    module_name: &str,
) -> Result<LocalModuleImport, ExtractionError> {
    // First check for new style module file (module.rs)
    let rs_path = directory_path.join(format!("{}.rs", module_name));
    if rs_path.exists() {
        // Check if this is a directory module (has submodules)
        let module_dir = directory_path.join(module_name);
        if module_dir.is_dir() {
            return Ok(LocalModuleImport {
                path: rs_path.to_string_lossy().to_string(),
                module_type: LocalModuleType::Directory(module_dir.to_string_lossy().to_string()),
            });
        }
        return Ok(LocalModuleImport {
            path: rs_path.to_string_lossy().to_string(),
            module_type: LocalModuleType::File,
        });
    }

    // Then check for old style module directory (module/mod.rs)
    let mod_rs_path = directory_path.join(module_name).join("mod.rs");
    if mod_rs_path.exists() {
        let module_dir = directory_path.join(module_name);
        return Ok(LocalModuleImport {
            path: mod_rs_path.to_string_lossy().to_string(),
            module_type: LocalModuleType::Directory(module_dir.to_string_lossy().to_string()),
        });
    }

    Err(ExtractionError::Malformed(format!(
        "Could not find module {} from {}",
        module_name,
        current_file.display()
    )))
}

fn prefix_namespace(name: &str, namespace: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", namespace, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::setup_parser;
    use assertables::assert_matches;
    use daipendency_testing::tempdir::TempDir;

    #[test]
    fn non_existing_file() {
        let path = PathBuf::from("non-existing.rs");
        let mut parser = setup_parser();

        let result = collect_module_directories(&path, &mut parser);

        assert!(matches!(result, Err(ExtractionError::Io(_))))
    }

    #[test]
    fn cyclic_modules() {
        let temp_dir = TempDir::new();
        let module_a_rs = temp_dir
            .create_file(
                "src/module_a.rs",
                r#"
pub mod module_b;
pub fn module_a_function() {}
"#,
            )
            .unwrap();
        temp_dir
            .create_file(
                "src/module_b.rs",
                r#"
pub mod module_a;  // This creates a cycle
pub fn module_b_function() {}
"#,
            )
            .unwrap();
        let mut parser = setup_parser();

        // This should complete without infinite recursion
        let directories = collect_module_directories(&module_a_rs, &mut parser).unwrap();

        assert!(!directories.is_empty())
    }

    #[test]
    fn root_module_directory_visibility() {
        let temp_dir = TempDir::new();
        let lib_rs = temp_dir
            .create_file(
                "src/lib.rs",
                r#"
pub fn public_function() {}
"#,
            )
            .unwrap();
        let mut parser = setup_parser();

        let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

        assert_eq!(directories.len(), 1);
        assert!(directories[0].is_public)
    }

    mod exports {
        use super::*;
        use assertables::assert_matches;

        #[test]
        fn public_symbol() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"
pub fn public_function() {}
"#,
                )
                .unwrap();
            let mut parser = setup_parser();

            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 1);
            assert_eq!(directories[0].name, "");
            assert_eq!(directories[0].entry_point.symbols.len(), 1);

            let definitions = &directories[0].entry_point.symbols;
            assert!(matches!(
                &definitions[0],
                RustSymbol::Symbol { symbol } if symbol.name == "public_function"
            ))
        }

        #[test]
        fn private_symbol() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"
fn private_function() {}
"#,
                )
                .unwrap();
            let mut parser = setup_parser();

            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 1);
            assert_eq!(directories[0].name, "");
            assert_eq!(directories[0].entry_point.symbols.len(), 0)
        }

        #[test]
        fn public_module() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"
pub mod public_module {}
"#,
                )
                .unwrap();
            let mut parser = setup_parser();

            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 1);
            let root = directories.get(0).unwrap();
            assert_eq!(root.name, "");
            assert_eq!(root.entry_point.symbols.len(), 1);
            assert_matches!(
                &root.entry_point.symbols[0],
                RustSymbol::ModuleBlock { name, is_public: true, doc_comment: None, .. }
                if name == "public_module"
            )
        }

        #[test]
        fn private_module() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"
mod private_module {}
"#,
                )
                .unwrap();
            let mut parser = setup_parser();

            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 1);
            let root = directories.get(0).unwrap();
            assert_eq!(root.name, "");
            assert_eq!(root.entry_point.symbols.len(), 1);
            assert_matches!(
                &root.entry_point.symbols[0],
                RustSymbol::ModuleBlock {
                    is_public: false,
                    ..
                }
            )
        }
    }

    mod reexports {
        use super::*;
        use crate::api::parsing::ImportType;
        use crate::api::test_helpers::get_module_directory;

        #[test]
        fn module_reexport() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"
pub mod module;
"#,
                )
                .unwrap();
            temp_dir
                .create_file(
                    "src/module.rs",
                    r#"
pub struct InnerStruct;
"#,
                )
                .unwrap();

            let mut parser = setup_parser();
            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 1);
            let root = get_module_directory("", &directories).unwrap();
            assert_eq!(root.entry_point.symbols.len(), 1);
            assert_matches!(
                &root.entry_point.symbols[0],
                RustSymbol::ModuleImport { name, is_reexported: true }
                if name == "module"
            );

            let module_file = root.internal_files.get("module").unwrap();
            assert_eq!(module_file.symbols.len(), 1);
            assert_matches!(
                &module_file.symbols[0],
                RustSymbol::Symbol { symbol } if symbol.name == "InnerStruct"
            )
        }

        #[test]
        fn direct_symbol_reexport() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"
mod formatter;
pub use formatter::Format;
"#,
                )
                .unwrap();
            temp_dir
                .create_file(
                    "src/formatter.rs",
                    r#"
pub enum Format {
    Plain,
    Rich,
}
"#,
                )
                .unwrap();
            let mut parser = setup_parser();

            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 1);
            let root = get_module_directory("", &directories).unwrap();
            assert_eq!(root.entry_point.symbols.len(), 2);

            assert_matches!(
                &root.entry_point.symbols[0],
                RustSymbol::ModuleImport { name, is_reexported: false }
                if name == "formatter"
            );
            assert_matches!(
                &root.entry_point.symbols[1],
                RustSymbol::Reexport { source_path, import_type }
                if source_path == "formatter::Format" && matches!(import_type, ImportType::Simple)
            );

            let formatter_file = root.internal_files.get("formatter").unwrap();
            assert_eq!(formatter_file.symbols.len(), 1);
            assert_matches!(
                &formatter_file.symbols[0],
                RustSymbol::Symbol { symbol } if symbol.name == "Format"
            )
        }

        #[test]
        fn indirect_symbol_reexport() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"
    mod formatting;
    pub use formatting::Format;
    "#,
                )
                .unwrap();
            temp_dir
                .create_file(
                    "src/formatting/mod.rs",
                    r#"
    mod format;
    pub use format::Format;
    "#,
                )
                .unwrap();
            temp_dir
                .create_file(
                    "src/formatting/format.rs",
                    r#"
    pub enum Format { Markdown, Html }
    "#,
                )
                .unwrap();
            let mut parser = setup_parser();

            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 2);
            let root = get_module_directory("", &directories).unwrap();
            assert_eq!(root.entry_point.symbols.len(), 2);
            assert_matches!(
                &root.entry_point.symbols[0],
                RustSymbol::ModuleImport { name, is_reexported: false }
                if name == "formatting"
            );
            assert_matches!(
                &root.entry_point.symbols[1],
                RustSymbol::Reexport { source_path, import_type }
                if source_path == "formatting::Format" && matches!(import_type, ImportType::Simple)
            );

            let formatting = get_module_directory("formatting", &directories).unwrap();
            assert!(!formatting.is_public);
            assert_eq!(formatting.entry_point.symbols.len(), 2);
            assert_matches!(
                &formatting.entry_point.symbols[0],
                RustSymbol::ModuleImport { name, is_reexported: false }
                if name == "format"
            );
            assert_matches!(
                &formatting.entry_point.symbols[1],
                RustSymbol::Reexport { source_path, import_type }
                if source_path == "format::Format" && matches!(import_type, ImportType::Simple)
            );

            let format_file = formatting.internal_files.get("format").unwrap();
            assert_eq!(format_file.symbols.len(), 1);
            assert_matches!(
                &format_file.symbols[0],
                RustSymbol::Symbol { symbol } if symbol.name == "Format"
            );
        }

        #[test]
        fn nested_modules_symbol_reexport() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"
pub mod child {
    pub mod grandchild {
        pub enum Format { Plain, Rich }
    }
}

pub use child::grandchild::Format;
"#,
                )
                .unwrap();
            let mut parser = setup_parser();

            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 1);
            let root = &directories[0];
            assert_eq!(root.entry_point.symbols.len(), 2);
            assert!(matches!(
                &root.entry_point.symbols[0],
                RustSymbol::ModuleBlock { name, is_public: true, content: child_content, doc_comment: None }
                if name == "child" &&
                matches!(&child_content[0], RustSymbol::ModuleBlock { name, is_public: true, content: grandchild_content, doc_comment: None } if name == "grandchild" &&
                  matches!(&grandchild_content[0], RustSymbol::Symbol { symbol } if symbol.name == "Format")
                  )
            ));
            assert_matches!(
                &root.entry_point.symbols[1],
                RustSymbol::Reexport { source_path, import_type: ImportType::Simple }
                if source_path == "child::grandchild::Format"
            )
        }

        #[test]
        fn wildcard_reexport() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"
    mod module;
    pub use module::*;
    "#,
                )
                .unwrap();
            temp_dir
                .create_file(
                    "src/module.rs",
                    r#"
    pub struct InnerStruct;
    "#,
                )
                .unwrap();
            let mut parser = setup_parser();

            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 1);
            let root = get_module_directory("", &directories).unwrap();
            assert_eq!(root.entry_point.symbols.len(), 2);
            assert_matches!(
                &root.entry_point.symbols[0],
                RustSymbol::ModuleImport { name, is_reexported: false }
                if name == "module"
            );
            assert_matches!(
                &root.entry_point.symbols[1],
                RustSymbol::Reexport { source_path, import_type }
                if source_path == "module" && matches!(import_type, ImportType::Wildcard)
            );

            let module_file = root.internal_files.get("module").unwrap();
            assert_eq!(module_file.symbols.len(), 1);
            assert_matches!(
                &module_file.symbols[0],
                RustSymbol::Symbol { symbol } if symbol.name == "InnerStruct"
            )
        }

        #[test]
        fn aliased_reexport() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"
    mod submodule;
    pub use submodule::Foo as Bar;
    "#,
                )
                .unwrap();
            temp_dir
                .create_file(
                    "src/submodule.rs",
                    r#"
    pub struct Foo;
    "#,
                )
                .unwrap();
            let mut parser = setup_parser();

            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 1);
            let root = get_module_directory("", &directories).unwrap();
            assert_eq!(root.entry_point.symbols.len(), 2);
            assert_matches!(
                &root.entry_point.symbols[0],
                RustSymbol::ModuleImport { name, is_reexported: false }
                if name == "submodule"
            );
            assert_matches!(
                &root.entry_point.symbols[1],
                RustSymbol::Reexport { source_path, import_type }
                if source_path == "submodule::Foo" && matches!(import_type, ImportType::Aliased(alias) if alias == "Bar")
            );

            let submodule_file = root.internal_files.get("submodule").unwrap();
            assert_eq!(submodule_file.symbols.len(), 1);
            assert_matches!(
                &submodule_file.symbols[0],
                RustSymbol::Symbol { symbol } if symbol.name == "Foo"
            )
        }

        #[test]
        fn file_with_mod_in_name() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"
    mod my_mod;
    pub use my_mod::MyStruct;
    "#,
                )
                .unwrap();
            temp_dir
                .create_file(
                    "src/my_mod.rs",
                    r#"
    pub struct MyStruct;
    "#,
                )
                .unwrap();

            let mut parser = setup_parser();
            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 1);
            let root = get_module_directory("", &directories).unwrap();
            assert_eq!(root.entry_point.symbols.len(), 2);
            assert_matches!(
                &root.entry_point.symbols[0],
                RustSymbol::ModuleImport { name, is_reexported: false } if name == "my_mod"
            );
            assert_matches!(
                &root.entry_point.symbols[1],
                RustSymbol::Reexport { source_path, import_type }
                if source_path == "my_mod::MyStruct" && matches!(import_type, ImportType::Simple)
            );

            let my_mod_file = root.internal_files.get("my_mod").unwrap();
            assert_eq!(my_mod_file.symbols.len(), 1);
            assert_matches!(
                &my_mod_file.symbols[0],
                RustSymbol::Symbol { symbol } if symbol.name == "MyStruct"
            )
        }
    }

    mod doc_comments {
        use super::*;

        #[test]
        fn file_with_doc_comment() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"//! This is a file-level doc comment.
//! It can span multiple lines.

pub struct Test {}
"#,
                )
                .unwrap();

            let mut parser = setup_parser();
            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 1);
            assert_eq!(directories[0].name, "");
            assert_eq!(
                directories[0].entry_point.doc_comment,
                Some(
                    "//! This is a file-level doc comment.\n//! It can span multiple lines.\n"
                        .to_string()
                )
            )
        }

        #[test]
        fn module_with_inner_doc_comment() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file(
                    "src/lib.rs",
                    r#"
pub mod inner {
    //! This is the inner doc comment
}
"#,
                )
                .unwrap();

            let mut parser = setup_parser();
            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 1);
            let root = directories.get(0).unwrap();
            assert_eq!(root.name, "");
            assert_eq!(root.entry_point.symbols.len(), 1);
            assert_matches!(
                &root.entry_point.symbols[0],
                RustSymbol::ModuleBlock { name, is_public: true, doc_comment, .. }
                if name == "inner" && *doc_comment == Some("//! This is the inner doc comment\n".to_string())
            )
        }
    }

    mod nested_module_directories {
        use super::*;
        use crate::api::test_helpers::get_module_directory;

        #[test]
        fn old_style() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file("src/lib.rs", r#"mod module;"#)
                .unwrap();
            temp_dir
                .create_file("src/module/mod.rs", r#"mod submodule;"#)
                .unwrap();
            temp_dir
                .create_file("src/module/submodule.rs", r#"pub struct SubStruct;"#)
                .unwrap();
            let mut parser = setup_parser();

            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 2);
            assert!(get_module_directory("", &directories).is_some());
            let module = get_module_directory("module", &directories).unwrap();
            assert!(module.internal_files.contains_key("submodule"));
            let submodule = module.internal_files.get("submodule").unwrap();
            assert_eq!(submodule.symbols.len(), 1);
            assert_matches!(
                &submodule.symbols[0],
                RustSymbol::Symbol { symbol } if symbol.name == "SubStruct"
            )
        }

        #[test]
        fn new_style() {
            let temp_dir = TempDir::new();
            let lib_rs = temp_dir
                .create_file("src/lib.rs", r#"mod module;"#)
                .unwrap();
            temp_dir
                .create_file("src/module.rs", r#"mod submodule;"#)
                .unwrap();
            temp_dir
                .create_file("src/module/submodule.rs", r#"pub struct SubStruct;"#)
                .unwrap();
            let mut parser = setup_parser();

            let directories = collect_module_directories(&lib_rs, &mut parser).unwrap();

            assert_eq!(directories.len(), 2);
            assert!(get_module_directory("", &directories).is_some());
            let module = get_module_directory("module", &directories).unwrap();
            assert!(module.internal_files.contains_key("submodule"));
            let submodule = module.internal_files.get("submodule").unwrap();
            assert_eq!(submodule.symbols.len(), 1);
            assert_matches!(
                &submodule.symbols[0],
                RustSymbol::Symbol { symbol } if symbol.name == "SubStruct"
            )
        }
    }
}
