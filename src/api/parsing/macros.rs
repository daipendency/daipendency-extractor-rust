use super::doc_comments::extract_outer_doc_comments;
use daipendency_extractor::ExtractionError;
use tree_sitter::Node;

pub fn get_macro_source_code(
    node: Node,
    source_code: &str,
) -> Result<Option<String>, ExtractionError> {
    let mut result = String::new();

    if let Some(doc_comment) = extract_outer_doc_comments(&node, source_code)? {
        result.push_str(&doc_comment);
    }

    let mut is_exported = false;
    let mut prev_sibling = node.prev_sibling();
    while let Some(sibling) = prev_sibling {
        if sibling.kind() == "attribute_item" {
            let attr_text = sibling
                .utf8_text(source_code.as_bytes())
                .map_err(|_| ExtractionError::Parse("Failed to read attribute text".to_string()))?;
            if attr_text == "#[macro_export]" {
                is_exported = true;
                result.push_str(attr_text);
                result.push('\n');
                break;
            }
        }
        prev_sibling = sibling.prev_sibling();
    }
    if !is_exported {
        return Ok(None);
    }

    let mut cursor = node.walk();
    let brace = node
        .children(&mut cursor)
        .find(|n| n.kind() == "{")
        .ok_or_else(|| ExtractionError::Parse("Failed to find macro body".to_string()))?;

    result.push_str(source_code[node.start_byte()..brace.start_byte()].trim_end());
    result.push(';');

    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{api::parsing::test_helpers::make_tree, treesitter_test_helpers::find_child_node};

    #[test]
    fn public_macro() {
        let source_code = r#"#[macro_export]
macro_rules! test_macro {
    () => { println!("Hello, world!"); }
}"#;
        let tree = make_tree(source_code);
        let macro_node = find_child_node(tree.root_node(), "macro_definition");

        let result = get_macro_source_code(macro_node, source_code).unwrap();

        assert_eq!(
            result,
            Some("#[macro_export]\nmacro_rules! test_macro;".to_string())
        );
    }

    #[test]
    fn private_macro() {
        let source_code = r#"macro_rules! test_macro {
    () => { println!("Hello, world!"); }
}"#;
        let tree = make_tree(source_code);
        let macro_node = find_child_node(tree.root_node(), "macro_definition");

        let result = get_macro_source_code(macro_node, source_code).unwrap();

        assert_eq!(result, None);
    }

    #[test]
    fn macro_with_doc_comment() {
        let source_code = r#"#[macro_export]
/// This is a test macro
macro_rules! test_macro {
    () => { println!("Hello, world!"); }
}"#;
        let tree = make_tree(source_code);
        let macro_node = find_child_node(tree.root_node(), "macro_definition");

        let result = get_macro_source_code(macro_node, source_code).unwrap();

        assert_eq!(
            result,
            Some("/// This is a test macro\n#[macro_export]\nmacro_rules! test_macro;".to_string())
        );
    }
}
