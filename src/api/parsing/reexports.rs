use super::files::{ImportType, RustSymbol};
use super::helpers::is_public;
use daipendency_extractor::ExtractionError;
use tree_sitter::Node;

pub fn extract_symbol_reexports(
    use_declaration_node: &Node,
    source_code: &str,
) -> Result<Vec<RustSymbol>, ExtractionError> {
    if !is_public(use_declaration_node) {
        return Ok(Vec::new());
    }

    let mut cursor = use_declaration_node.walk();
    let children: Vec<_> = use_declaration_node.children(&mut cursor).collect();

    let result = if let Some(scoped) = children.iter().find(|c| c.kind() == "scoped_identifier") {
        extract_single_reexport(scoped, source_code)
    } else if let Some(use_as) = children.iter().find(|c| c.kind() == "use_as_clause") {
        extract_renamed_reexport(use_as, source_code)
    } else if let Some(scoped_list) = children.iter().find(|c| c.kind() == "scoped_use_list") {
        extract_multi_reexports(scoped_list, source_code)
    } else if let Some(wildcard) = children.iter().find(|c| c.kind() == "use_wildcard") {
        extract_wildcard_reexport(wildcard, source_code)
    } else if let Some(identifier) = children.iter().find(|c| c.kind() == "identifier") {
        extract_external_crate_reexport(identifier, source_code)
    } else {
        Err(ExtractionError::Malformed(format!(
            "Failed to find symbol reexport: {}",
            use_declaration_node
                .utf8_text(source_code.as_bytes())
                .unwrap()
        )))
    };

    result.map(normalize_raw_identifiers)
}

fn extract_external_crate_reexport(
    identifier: &Node,
    source_code: &str,
) -> Result<Vec<RustSymbol>, ExtractionError> {
    let source_path = identifier
        .utf8_text(source_code.as_bytes())
        .map_err(|e| ExtractionError::Malformed(e.to_string()))?
        .to_string();
    Ok(vec![RustSymbol::Reexport {
        source_path,
        import_type: ImportType::Simple,
    }])
}

fn extract_wildcard_reexport(
    wildcard: &Node,
    source_code: &str,
) -> Result<Vec<RustSymbol>, ExtractionError> {
    let mut cursor = wildcard.walk();
    let children: Vec<_> = wildcard.children(&mut cursor).collect();

    let module_path = children
        .iter()
        .find(|c| c.kind() == "identifier" || c.kind() == "scoped_identifier")
        .ok_or_else(|| {
            ExtractionError::Malformed(format!(
                "Failed to find module path in wildcard import: {}",
                wildcard
                    .utf8_text(source_code.as_bytes())
                    .unwrap_or_default()
            ))
        })?
        .utf8_text(source_code.as_bytes())
        .map_err(|e| ExtractionError::Malformed(e.to_string()))?;

    Ok(vec![RustSymbol::Reexport {
        source_path: module_path.to_string(),
        import_type: ImportType::Wildcard,
    }])
}

fn extract_single_reexport(
    scoped: &Node,
    source_code: &str,
) -> Result<Vec<RustSymbol>, ExtractionError> {
    let mut cursor = scoped.walk();
    let source_path = scoped
        .children(&mut cursor)
        .map(|child| {
            child
                .utf8_text(source_code.as_bytes())
                .map_err(|e| ExtractionError::Malformed(e.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("");
    Ok(vec![RustSymbol::Reexport {
        source_path,
        import_type: ImportType::Simple,
    }])
}

fn extract_renamed_reexport(
    use_as: &Node,
    source_code: &str,
) -> Result<Vec<RustSymbol>, ExtractionError> {
    let mut cursor = use_as.walk();
    let children: Vec<_> = use_as.children(&mut cursor).collect();

    let source_path = children
        .first()
        .ok_or_else(|| ExtractionError::Malformed("Empty use_as clause".to_string()))?
        .utf8_text(source_code.as_bytes())
        .map_err(|e| ExtractionError::Malformed(e.to_string()))?
        .to_string();

    let alias = children
        .iter()
        .find(|c| c.kind() == "identifier")
        .ok_or_else(|| ExtractionError::Malformed("No alias found in use_as clause".to_string()))?
        .utf8_text(source_code.as_bytes())
        .map_err(|e| ExtractionError::Malformed(e.to_string()))?
        .to_string();

    Ok(vec![RustSymbol::Reexport {
        source_path,
        import_type: ImportType::Aliased(alias),
    }])
}

fn extract_multi_reexports(
    scoped_list: &Node,
    source_code: &str,
) -> Result<Vec<RustSymbol>, ExtractionError> {
    let mut scoped_cursor = scoped_list.walk();
    let scoped_children: Vec<_> = scoped_list.children(&mut scoped_cursor).collect();

    let path_prefix = scoped_children
        .first()
        .ok_or_else(|| ExtractionError::Malformed("Empty scoped list".to_string()))?
        .utf8_text(source_code.as_bytes())
        .map_err(|e| ExtractionError::Malformed(e.to_string()))?
        .to_string();

    let use_list = scoped_children
        .iter()
        .find(|c| c.kind() == "use_list")
        .ok_or_else(|| ExtractionError::Malformed("No use list found".to_string()))?;

    let mut list_cursor = use_list.walk();
    use_list
        .children(&mut list_cursor)
        .filter(|item| item.kind() == "identifier")
        .map(|item| {
            let name = item
                .utf8_text(source_code.as_bytes())
                .map_err(|e| ExtractionError::Malformed(e.to_string()))?;
            Ok(RustSymbol::Reexport {
                source_path: format!("{}::{}", path_prefix, name),
                import_type: ImportType::Simple,
            })
        })
        .collect()
}

fn normalize_raw_identifiers(symbols: Vec<RustSymbol>) -> Vec<RustSymbol> {
    symbols
        .into_iter()
        .map(|symbol| match symbol {
            RustSymbol::Reexport {
                source_path,
                import_type,
            } => {
                let normalized_path = source_path
                    .split("::")
                    .map(|part| part.trim_start_matches("r#").to_string())
                    .collect::<Vec<_>>()
                    .join("::");

                let normalized_type = match import_type {
                    ImportType::Aliased(alias) => {
                        ImportType::Aliased(alias.trim_start_matches("r#").to_string())
                    }
                    other => other,
                };

                RustSymbol::Reexport {
                    source_path: normalized_path,
                    import_type: normalized_type,
                }
            }
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::parsing::test_helpers::make_tree;
    use crate::treesitter_test_helpers::find_child_node;
    use assertables::{assert_contains, assert_matches, assert_ok};

    fn get_reexports(symbols: &[RustSymbol]) -> Vec<String> {
        symbols
            .iter()
            .filter_map(|s| match s {
                RustSymbol::Reexport { source_path, .. } => Some(source_path.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn non_import() {
        let source_code = r#"pub enum Format {}"#;
        let tree = make_tree(source_code);
        let root_node = tree.root_node();
        let use_declaration = find_child_node(root_node, "enum_item");

        let result = extract_symbol_reexports(&use_declaration, source_code);

        assert_matches!(
            result.unwrap_err(),
            ExtractionError::Malformed(msg) if msg == format!("Failed to find symbol reexport: {}", source_code)
        );
    }

    #[test]
    fn external_crate_reexport() {
        let source_code = r#"pub use serde_json;"#;
        let tree = make_tree(source_code);
        let root_node = tree.root_node();
        let use_declaration = find_child_node(root_node, "use_declaration");

        let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

        assert_eq!(symbols.len(), 1);
        assert_matches!(
            &symbols[0],
            RustSymbol::Reexport { source_path, import_type: ImportType::Simple } if source_path == "serde_json"
        );
    }

    #[test]
    fn import_without_reexport() {
        let source_code = r#"
use inner::Format;
"#;
        let tree = make_tree(source_code);
        let use_declaration = find_child_node(tree.root_node(), "use_declaration");

        let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

        assert!(symbols.is_empty());
    }

    #[test]
    fn single_reexport() {
        let source_code = r#"
pub use inner::Format;
"#;
        let tree = make_tree(source_code);
        let use_declaration = find_child_node(tree.root_node(), "use_declaration");

        let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

        let reexports = get_reexports(&symbols);
        assert_contains!(&reexports, &"inner::Format".to_string());
    }

    #[test]
    fn renamed_reexport() {
        let source_code = r#"pub use inner::Foo as Bar;"#;
        let tree = make_tree(source_code);
        let use_declaration = find_child_node(tree.root_node(), "use_declaration");

        let result = extract_symbol_reexports(&use_declaration, source_code);

        assert_ok!(&result);
        let symbols = result.unwrap();
        assert_eq!(symbols.len(), 1);
        assert_matches!(
            &symbols[0],
            RustSymbol::Reexport {
                source_path,
                import_type: ImportType::Aliased(alias)
            } if source_path == "inner::Foo" && alias == "Bar"
        );
    }

    #[test]
    fn multiple_reexports() {
        let source_code = r#"
pub use inner::{TextFormatter, OtherType};
"#;
        let tree = make_tree(source_code);
        let use_declaration = find_child_node(tree.root_node(), "use_declaration");

        let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

        let reexports = get_reexports(&symbols);
        assert_contains!(&reexports, &"inner::TextFormatter".to_string());
        assert_contains!(&reexports, &"inner::OtherType".to_string());
    }

    #[test]
    fn relative_wildcard_reexport() {
        let source_code = r#"
pub use inner::*;
"#;
        let tree = make_tree(source_code);
        let use_declaration = find_child_node(tree.root_node(), "use_declaration");

        let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

        assert_eq!(symbols.len(), 1);
        assert_matches!(
            &symbols[0],
            RustSymbol::Reexport {
                source_path,
                import_type: ImportType::Wildcard,
            } if source_path == "inner"
        );
    }

    #[test]
    fn absolute_wildcard_reexport() {
        let source_code = r#"
pub use crate::inner::*;
"#;
        let tree = make_tree(source_code);
        let use_declaration = find_child_node(tree.root_node(), "use_declaration");

        let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

        assert_eq!(symbols.len(), 1);
        assert_matches!(
            &symbols[0],
            RustSymbol::Reexport {
                source_path,
                import_type: ImportType::Wildcard,
            } if source_path == "crate::inner"
        );
    }

    mod raw_identifiers {
        use super::*;

        #[test]
        fn module_import() {
            let source_code = r#"pub use r#type;"#;
            let tree = make_tree(source_code);
            let use_declaration = find_child_node(tree.root_node(), "use_declaration");

            let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

            assert_eq!(symbols.len(), 1);
            assert_matches!(
                &symbols[0],
                RustSymbol::Reexport { source_path, import_type: ImportType::Simple } if source_path == "type"
            );
        }

        #[test]
        fn simple_symbol_import() {
            let source_code = r#"pub use submodule::r#fn;"#;
            let tree = make_tree(source_code);
            let use_declaration = find_child_node(tree.root_node(), "use_declaration");

            let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

            assert_eq!(symbols.len(), 1);
            assert_matches!(
                &symbols[0],
                RustSymbol::Reexport { source_path, import_type: ImportType::Simple } if source_path == "submodule::fn"
            );
        }

        #[test]
        fn alias_module_as_raw() {
            let source_code = r#"pub use submodule::the_type as r#type;"#;
            let tree = make_tree(source_code);
            let use_declaration = find_child_node(tree.root_node(), "use_declaration");

            let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

            assert_eq!(symbols.len(), 1);
            assert_matches!(
                &symbols[0],
                RustSymbol::Reexport { source_path, import_type: ImportType::Aliased(alias) }
                if source_path == "submodule::the_type" && alias == "type"
            );
        }

        #[test]
        fn alias_raw_as_module() {
            let source_code = r#"pub use r#type::Foo as Bar;"#;
            let tree = make_tree(source_code);
            let use_declaration = find_child_node(tree.root_node(), "use_declaration");

            let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

            assert_eq!(symbols.len(), 1);
            assert_matches!(
                &symbols[0],
                RustSymbol::Reexport { source_path, import_type: ImportType::Aliased(alias) }
                if source_path == "type::Foo" && alias == "Bar"
            );
        }

        #[test]
        fn raw_module_wildcard() {
            let source_code = r#"pub use r#type::{Foo, Bar};"#;
            let tree = make_tree(source_code);
            let use_declaration = find_child_node(tree.root_node(), "use_declaration");

            let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

            let reexports = get_reexports(&symbols);
            assert_contains!(&reexports, &"type::Foo".to_string());
            assert_contains!(&reexports, &"type::Bar".to_string());
        }

        #[test]
        fn module_raw_symbols_wildcard() {
            let source_code = r#"pub use submodule::{r#fn, r#type};"#;
            let tree = make_tree(source_code);
            let use_declaration = find_child_node(tree.root_node(), "use_declaration");

            let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

            let reexports = get_reexports(&symbols);
            assert_contains!(&reexports, &"submodule::fn".to_string());
            assert_contains!(&reexports, &"submodule::type".to_string());
        }
    }
}
