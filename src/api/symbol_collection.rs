use daipendency_extractor::ExtractionError;
use daipendency_extractor::Symbol;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tree_sitter::Parser;

use super::parsing;
use super::parsing::ImportType;

#[derive(Debug, Clone)]
pub struct ModuleContents {
    pub definitions: Vec<Symbol>,
    pub references: Vec<Reference>,
}

/// A module is a collection of symbols and references.
///
/// It can represent:
/// - A crate's root, where `src/lib.rs` is the entry point and other files in `src/*.rs` are internal.
/// - A submodule, where `src/submodule/mod.rs` is the entry point and other files in `src/submodule/*.rs` are internal.
/// - A `mod submodule {...}` block, where the entry point is the contents of the block (symbol declarations and reexports), and there are no internal files.
#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub entry_point: ModuleContents,
    pub internal_files: HashMap<String, ModuleContents>,
    pub is_public: bool,
    pub doc_comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reference {
    /// A symbol that is reexported directly (e.g. `pub use submodule::Foo`).
    Symbol(String),
    /// A symbol that is reexported with an alias (e.g. `pub use submodule::Foo as Bar`).
    AliasedSymbol {
        /// The original symbol that is reexported (e.g. `submodule::Foo`).
        source_path: String,
        /// The alias that the original symbol is reexported as (e.g. `Bar`).
        alias: String,
    },
    /// A symbol that is reexported with a wildcard (e.g. `pub use submodule::*`).
    Wildcard(String),
}

/// Traverse the source files of the Rust crate and collect all symbols and symbol references (reexports).
pub fn collect_symbols(
    entry_point: &Path,
    parser: &mut Parser,
) -> Result<Vec<Module>, ExtractionError> {
    let mut visited_files = HashMap::new();
    collect_symbols_recursively(entry_point, "", true, parser, &mut visited_files)
}

fn collect_symbols_recursively(
    file_path: &Path,
    namespace_prefix: &str,
    is_public: bool,
    parser: &mut Parser,
    visited_files: &mut HashMap<PathBuf, bool>,
) -> Result<Vec<Module>, ExtractionError> {
    if visited_files.contains_key(&file_path.to_path_buf()) {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(file_path).map_err(ExtractionError::Io)?;

    visited_files.insert(file_path.to_path_buf(), true);
    let rust_file = parsing::parse_rust_file(&content, parser)?;

    collect_module_symbols(
        rust_file.symbols,
        namespace_prefix,
        is_public,
        file_path,
        parser,
        visited_files,
        rust_file.doc_comment,
    )
}

fn collect_module_symbols(
    content: Vec<parsing::RustSymbol>,
    namespace_prefix: &str,
    is_public: bool,
    file_path: &Path,
    parser: &mut Parser,
    visited_files: &mut HashMap<PathBuf, bool>,
    doc_comment: Option<String>,
) -> Result<Vec<Module>, ExtractionError> {
    let mut modules = Vec::new();
    let mut current_namespace = Module {
        name: namespace_prefix.to_string(),
        entry_point: ModuleContents {
            definitions: Vec::new(),
            references: Vec::new(),
        },
        internal_files: HashMap::new(),
        is_public,
        doc_comment,
    };

    for symbol in content {
        match symbol {
            parsing::RustSymbol::Symbol { symbol } => {
                current_namespace
                    .entry_point
                    .definitions
                    .push(symbol.clone());
            }
            parsing::RustSymbol::Module {
                name,
                content,
                doc_comment,
            } => {
                let module_namespace = prefix_namespace(&name, namespace_prefix);
                let mut nested_modules = collect_module_symbols(
                    content,
                    &module_namespace,
                    is_public,
                    file_path,
                    parser,
                    visited_files,
                    doc_comment,
                )?;
                modules.append(&mut nested_modules);
            }
            parsing::RustSymbol::ModuleImport {
                name,
                is_reexported: module_is_public,
                ..
            } => {
                if let Ok(module_path) = resolve_module_path(file_path, &name) {
                    // Check if this is a submodule (in a subdirectory) or a sibling file
                    let parent_path = file_path.parent().unwrap();
                    let submodule_path = parent_path.join(&name).join("mod.rs");
                    let is_submodule = module_path == submodule_path;
                    if is_submodule {
                        let module_namespace = prefix_namespace(&name, namespace_prefix);
                        let mut child_namespaces = collect_symbols_recursively(
                            &module_path,
                            &module_namespace,
                            module_is_public,
                            parser,
                            visited_files,
                        )?;
                        modules.append(&mut child_namespaces);
                    } else {
                        let mut child_namespaces = collect_symbols_recursively(
                            &module_path,
                            namespace_prefix, // Keep the parent's namespace
                            module_is_public,
                            parser,
                            visited_files,
                        )?;
                        if let Some(child) = child_namespaces.pop() {
                            current_namespace
                                .internal_files
                                .insert(name.clone(), child.entry_point);
                        }
                    }
                }
            }
            parsing::RustSymbol::SymbolReexport {
                source_path,
                import_type: reexport_type,
            } => {
                let source_path = prefix_namespace(&source_path, namespace_prefix);
                match reexport_type {
                    ImportType::Simple => {
                        current_namespace
                            .entry_point
                            .references
                            .push(Reference::Symbol(source_path));
                    }
                    ImportType::Wildcard => {
                        current_namespace
                            .entry_point
                            .references
                            .push(Reference::Wildcard(source_path));
                    }
                    ImportType::Aliased(alias_name) => {
                        current_namespace
                            .entry_point
                            .references
                            .push(Reference::AliasedSymbol {
                                source_path,
                                alias: alias_name,
                            });
                    }
                }
            }
        }
    }

    modules.push(current_namespace);
    Ok(modules)
}

fn resolve_module_path(current_file: &Path, module_name: &str) -> Result<PathBuf, ExtractionError> {
    let parent = current_file.parent().ok_or_else(|| {
        ExtractionError::Malformed(format!(
            "Failed to get parent directory of {}",
            current_file.display()
        ))
    })?;

    let mod_rs_path = parent.join(module_name).join("mod.rs");
    if mod_rs_path.exists() {
        return Ok(mod_rs_path);
    }

    let rs_path = parent.join(format!("{}.rs", module_name));
    if rs_path.exists() {
        return Ok(rs_path);
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
    use crate::test_helpers::{create_file, create_temp_dir};

    #[test]
    fn non_existing_file() {
        let path = PathBuf::from("non-existing.rs");
        let mut parser = setup_parser();

        let result = collect_symbols(&path, &mut parser);

        assert!(matches!(result, Err(ExtractionError::Io(_))));
    }

    #[test]
    fn cyclic_modules() {
        let temp_dir = create_temp_dir();
        let module_a_rs = temp_dir.path().join("src").join("module_a.rs");
        let module_b_rs = temp_dir.path().join("src").join("module_b.rs");
        create_file(
            &module_a_rs,
            r#"
pub mod module_b;
pub fn module_a_function() {}
"#,
        );
        create_file(
            &module_b_rs,
            r#"
pub mod module_a;  // This creates a cycle
pub fn module_b_function() {}
"#,
        );
        let mut parser = setup_parser();

        // This should complete without infinite recursion
        let modules = collect_symbols(&module_a_rs, &mut parser).unwrap();

        assert!(!modules.is_empty());
    }

    mod exports {
        use super::*;

        #[test]
        fn public_symbol() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            create_file(
                &lib_rs,
                r#"
pub fn public_function() {}
"#,
            );
            let mut parser = setup_parser();

            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 1);
            assert_eq!(modules[0].name, "");
            assert_eq!(modules[0].entry_point.definitions.len(), 1);

            let definitions = &modules[0].entry_point.definitions;
            assert!(definitions.iter().any(|s| s.name == "public_function"));
        }

        #[test]
        fn private_symbol() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            create_file(
                &lib_rs,
                r#"
fn private_function() {}
"#,
            );
            let mut parser = setup_parser();

            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 1);
            assert_eq!(modules[0].name, "");
            assert_eq!(modules[0].entry_point.definitions.len(), 0);
        }

        #[test]
        fn public_module() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            create_file(
                &lib_rs,
                r#"
pub mod public_module {}
"#,
            );
            let mut parser = setup_parser();

            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 2);
            assert!(modules.iter().any(|n| n.name == "public_module"));
        }

        #[test]
        fn private_module() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            create_file(
                &lib_rs,
                r#"
mod private_module {}
"#,
            );
            let mut parser = setup_parser();

            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 1);
            assert!(modules.iter().any(|n| n.name == ""));
        }
    }

    mod reexports {
        use crate::api::test_helpers::get_module;

        use super::*;

        #[test]
        fn missing_module() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            create_file(
                &lib_rs,
                r#"
    pub mod missing;
    "#,
            );
            let mut parser = setup_parser();

            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 1);
            assert_eq!(modules[0].name, "");
            assert_eq!(modules[0].entry_point.references.len(), 0);
        }

        #[test]
        fn module_reexport() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            let module_rs = temp_dir.path().join("src").join("module.rs");

            create_file(
                &lib_rs,
                r#"
pub mod module;
"#,
            );
            create_file(
                &module_rs,
                r#"
pub struct InnerStruct;
"#,
            );

            let mut parser = setup_parser();
            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 1);
            let root = get_module("", &modules).unwrap();
            assert_eq!(root.entry_point.definitions.len(), 0);
            assert_eq!(root.entry_point.references.len(), 0);

            let module_file = root.internal_files.get("module").unwrap();
            assert_eq!(module_file.definitions.len(), 1);
            assert_eq!(module_file.references.len(), 0);
            assert_eq!(module_file.definitions[0].name, "InnerStruct");
        }

        #[test]
        fn direct_symbol_reexport() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            let formatter_rs = temp_dir.path().join("src").join("formatter.rs");
            create_file(
                &lib_rs,
                r#"
mod formatter;
pub use formatter::Format;
"#,
            );
            create_file(
                &formatter_rs,
                r#"
pub enum Format {
    Plain,
    Rich,
}
"#,
            );
            let mut parser = setup_parser();

            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 1);
            let root = get_module("", &modules).unwrap();
            assert_eq!(root.entry_point.definitions.len(), 0);
            assert_eq!(root.entry_point.references.len(), 1);
            assert_eq!(
                root.entry_point.references[0],
                Reference::Symbol("formatter::Format".to_string())
            );

            let formatter_file = root.internal_files.get("formatter").unwrap();
            assert_eq!(formatter_file.definitions.len(), 1);
            assert!(formatter_file
                .definitions
                .iter()
                .any(|s| s.name == "Format"));
        }

        #[test]
        fn indirect_symbol_reexport() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            let formatting_dir = temp_dir.path().join("src").join("formatting");
            let formatting_mod_rs = formatting_dir.join("mod.rs");
            let format_rs = formatting_dir.join("format.rs");
            create_file(
                &lib_rs,
                r#"
    mod formatting;
    pub use formatting::Format;
    "#,
            );
            create_file(
                &formatting_mod_rs,
                r#"
    mod format;
    pub use format::Format;
    "#,
            );
            create_file(
                &format_rs,
                r#"
    pub enum Format {
        Markdown,
        Html,
        Plain,
    }
    "#,
            );
            let mut parser = setup_parser();

            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 2,);
            let root = get_module("", &modules).unwrap();
            assert_eq!(root.entry_point.definitions.len(), 0,);
            assert_eq!(root.entry_point.references.len(), 1,);
            assert_eq!(
                root.entry_point.references[0],
                Reference::Symbol("formatting::Format".to_string()),
            );

            let formatting = get_module("formatting", &modules).unwrap();
            assert_eq!(formatting.entry_point.definitions.len(), 0,);
            assert_eq!(formatting.entry_point.references.len(), 1,);
            assert_eq!(
                formatting.entry_point.references[0],
                Reference::Symbol("formatting::format::Format".to_string()),
            );

            let format_file = formatting.internal_files.get("format").unwrap();
            assert_eq!(format_file.definitions.len(), 1,);
            assert_eq!(format_file.references.len(), 0,);
            assert_eq!(format_file.definitions[0].name, "Format");
        }

        #[test]
        fn nested_modules_symbol_reexport() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            create_file(
                &lib_rs,
                r#"
pub mod child {
    pub mod grandchild {
        pub enum Format {
            Plain,
            Rich,
        }
    }
}
"#,
            );
            let mut parser = setup_parser();

            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            let grandchild = get_module("child::grandchild", &modules).unwrap();
            assert_eq!(grandchild.entry_point.definitions.len(), 1);
            let enum_definition = grandchild
                .entry_point
                .definitions
                .iter()
                .find(|s| s.name == "Format");
            assert!(enum_definition.is_some());
        }

        #[test]
        fn wildcard_reexport() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            let module_rs = temp_dir.path().join("src").join("module.rs");

            create_file(
                &lib_rs,
                r#"
    mod module;
    pub use module::*;
    "#,
            );
            create_file(
                &module_rs,
                r#"
    pub struct InnerStruct;
    "#,
            );

            let mut parser = setup_parser();
            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 1);
            let root = get_module("", &modules).unwrap();
            assert_eq!(root.entry_point.definitions.len(), 0);
            assert_eq!(root.entry_point.references.len(), 1);
            assert_eq!(
                root.entry_point.references[0],
                Reference::Wildcard("module".to_string())
            );

            let module_file = root.internal_files.get("module").unwrap();
            assert_eq!(module_file.definitions.len(), 1);
            assert_eq!(module_file.references.len(), 0);
            assert_eq!(module_file.definitions[0].name, "InnerStruct");
        }

        #[test]
        fn file_with_mod_in_name() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            let my_mod_rs = temp_dir.path().join("src").join("my_mod.rs");

            create_file(
                &lib_rs,
                r#"
    mod my_mod;
    pub use my_mod::MyStruct;
    "#,
            );
            create_file(
                &my_mod_rs,
                r#"
    pub struct MyStruct;
    "#,
            );

            let mut parser = setup_parser();
            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 1);
            let root = get_module("", &modules).unwrap();
            assert_eq!(root.entry_point.definitions.len(), 0);
            assert_eq!(root.entry_point.references.len(), 1);
            assert_eq!(
                root.entry_point.references[0],
                Reference::Symbol("my_mod::MyStruct".to_string())
            );

            let my_mod_file = root.internal_files.get("my_mod").unwrap();
            assert_eq!(my_mod_file.definitions.len(), 1);
            assert_eq!(my_mod_file.references.len(), 0);
            assert_eq!(my_mod_file.definitions[0].name, "MyStruct");
        }

        #[test]
        fn aliased_reexport() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            let submodule_rs = temp_dir.path().join("src").join("submodule.rs");

            create_file(
                &lib_rs,
                r#"
    mod submodule;
    pub use submodule::Foo as Bar;
    "#,
            );
            create_file(
                &submodule_rs,
                r#"
    pub struct Foo;
    "#,
            );

            let mut parser = setup_parser();
            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 1);
            let root = get_module("", &modules).unwrap();
            assert_eq!(root.entry_point.definitions.len(), 0);
            assert_eq!(root.entry_point.references.len(), 1);
            assert_eq!(
                root.entry_point.references[0],
                Reference::AliasedSymbol {
                    source_path: "submodule::Foo".to_string(),
                    alias: "Bar".to_string(),
                }
            );

            let submodule_file = root.internal_files.get("submodule").unwrap();
            assert_eq!(submodule_file.definitions.len(), 1);
            assert_eq!(submodule_file.references.len(), 0);
            assert_eq!(submodule_file.definitions[0].name, "Foo");
        }
    }

    mod doc_comments {
        use super::*;

        #[test]
        fn file_with_doc_comment() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            create_file(
                &lib_rs,
                r#"//! This is a file-level doc comment.
//! It can span multiple lines.

pub struct Test {}
"#,
            );

            let mut parser = setup_parser();
            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 1);
            assert_eq!(modules[0].name, "");
            assert_eq!(
                modules[0].doc_comment,
                Some(
                    "//! This is a file-level doc comment.\n//! It can span multiple lines.\n"
                        .to_string()
                )
            );
        }

        #[test]
        fn module_with_inner_doc_comment() {
            let temp_dir = create_temp_dir();
            let lib_rs = temp_dir.path().join("src").join("lib.rs");
            create_file(
                &lib_rs,
                r#"
pub mod inner {
    //! This is the inner doc comment
    //! It spans multiple lines

    pub fn nested_function() -> String {}
}
"#,
            );

            let mut parser = setup_parser();
            let modules = collect_symbols(&lib_rs, &mut parser).unwrap();

            assert_eq!(modules.len(), 2);
            let inner_namespace = modules.iter().find(|n| n.name == "inner").unwrap();
            assert_eq!(
                inner_namespace.doc_comment,
                Some(
                    "//! This is the inner doc comment\n//! It spans multiple lines\n".to_string()
                )
            );
        }
    }
}
