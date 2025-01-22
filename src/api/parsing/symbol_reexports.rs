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
    } else if let Some(scoped_list) = children.iter().find(|c| c.kind() == "scoped_use_list") {
        extract_multi_reexports(scoped_list, source_code)
    } else if let Some(wildcard) = children.iter().find(|c| c.kind() == "use_wildcard") {
        extract_wildcard_reexport(wildcard, source_code)
    } else {
        Err(ExtractionError::Malformed(
            "Failed to find symbol reexport".to_string(),
        ))
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
        .find(|c| c.kind() == "identifier")
        .ok_or_else(|| {
            ExtractionError::Malformed("Failed to find module path in wildcard import".to_string())
        })?
        .utf8_text(source_code.as_bytes())
        .map_err(|e| ExtractionError::Malformed(e.to_string()))?;
    Ok(vec![RustSymbol::SymbolReexport {
        source_path: module_path.to_string(),
        is_wildcard: true,
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
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::parsing::test_helpers::make_tree;
    use crate::treesitter_test_helpers::find_child_node;
    use assertables::assert_contains;

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
        let source_code = r#"
pub enum Format {}
"#;
        let tree = make_tree(source_code);
        let root_node = tree.root_node();
        let use_declaration = find_child_node(root_node, "enum_item");

        let result = extract_symbol_reexports(&use_declaration, source_code);

        assert!(matches!(
            result.unwrap_err(),
            ExtractionError::Malformed(msg) if msg == "Failed to find symbol reexport"
        ));
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
    fn wildcard_reexport() {
        let source_code = r#"
pub use inner::*;
"#;
        let tree = make_tree(source_code);
        let use_declaration = find_child_node(tree.root_node(), "use_declaration");

        let symbols = extract_symbol_reexports(&use_declaration, source_code).unwrap();

        assert_eq!(symbols.len(), 1);
        assert!(matches!(
            &symbols[0],
            RustSymbol::SymbolReexport { source_path, is_wildcard }
            if source_path == "inner" && *is_wildcard
        ));
    }
}
