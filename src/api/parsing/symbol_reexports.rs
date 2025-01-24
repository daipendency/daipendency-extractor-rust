use super::files::RustSymbol;
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

    if let Some(scoped) = children.iter().find(|c| c.kind() == "scoped_identifier") {
        Ok(vec![extract_single_reexport(scoped, source_code)?])
    } else if let Some(use_as) = children.iter().find(|c| c.kind() == "use_as_clause") {
        Ok(vec![extract_renamed_reexport(use_as, source_code)?])
    } else if let Some(scoped_list) = children.iter().find(|c| c.kind() == "scoped_use_list") {
        extract_multi_reexports(scoped_list, source_code)
    } else if let Some(wildcard) = children.iter().find(|c| c.kind() == "use_wildcard") {
        extract_wildcard_reexport(wildcard, source_code)
    } else {
        Err(ExtractionError::Malformed(format!(
            "Failed to find symbol reexport: {}",
            use_declaration_node
                .utf8_text(source_code.as_bytes())
                .unwrap()
        )))
    }
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

    Ok(vec![RustSymbol::SymbolReexport {
        source_path: module_path.to_string(),
        is_wildcard: true,
        alias: None,
    }])
}

fn extract_single_reexport(
    scoped: &Node,
    source_code: &str,
) -> Result<RustSymbol, ExtractionError> {
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
    Ok(RustSymbol::SymbolReexport {
        source_path,
        is_wildcard: false,
        alias: None,
    })
}

fn extract_renamed_reexport(
    use_as: &Node,
    source_code: &str,
) -> Result<RustSymbol, ExtractionError> {
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

    Ok(RustSymbol::SymbolReexport {
        source_path,
        is_wildcard: false,
        alias: Some(alias),
    })
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
            Ok(RustSymbol::SymbolReexport {
                source_path: format!("{}::{}", path_prefix, name),
                is_wildcard: false,
                alias: None,
            })
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
                RustSymbol::SymbolReexport { source_path, .. } => Some(source_path.clone()),
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
            RustSymbol::SymbolReexport {
                source_path,
                is_wildcard: false,
                alias: Some(alias)
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
            RustSymbol::SymbolReexport {
                source_path,
                is_wildcard,
                alias: None
            } if source_path == "inner" && *is_wildcard
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
            RustSymbol::SymbolReexport {
                source_path,
                is_wildcard,
                alias: None
            } if source_path == "crate::inner" && *is_wildcard
        );
    }
}
