use daipendency_extractor::LaibraryError;
use tree_sitter::Node;

#[derive(Debug, Clone, Copy)]
enum DocCommentMarker {
    Outer,
    Inner,
}

impl DocCommentMarker {
    fn kind(&self) -> &'static str {
        match self {
            DocCommentMarker::Outer => "outer_doc_comment_marker",
            DocCommentMarker::Inner => "inner_doc_comment_marker",
        }
    }
}

pub fn extract_outer_doc_comments(
    node: &Node,
    source_code: &str,
) -> Result<Option<String>, LaibraryError> {
    let Some(previous_sibling) = node.prev_sibling() else {
        return Ok(None);
    };

    let previous_sibling = skip_preceding_attributes(previous_sibling);

    if let Some(comment) = extract_preceding_block_comment(&previous_sibling, source_code)? {
        return Ok(Some(comment));
    }

    extract_preceding_line_doc_comments(previous_sibling, source_code)
}

fn skip_preceding_attributes(mut node: Node) -> Node {
    while node.kind() == "attribute_item" {
        if let Some(prev) = node.prev_sibling() {
            node = prev;
        } else {
            break;
        }
    }
    node
}

fn is_doc_comment(node: &Node, marker: DocCommentMarker) -> bool {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    children.iter().any(|c| c.kind() == marker.kind())
        && children.iter().any(|child| child.kind() == "doc_comment")
}

fn extract_preceding_block_comment(
    node: &Node,
    source_code: &str,
) -> Result<Option<String>, LaibraryError> {
    if node.kind() == "block_comment" && is_doc_comment(node, DocCommentMarker::Outer) {
        let text = node
            .utf8_text(source_code.as_bytes())
            .map_err(|e| LaibraryError::Parse(e.to_string()))?;
        return Ok(Some(text.to_string() + "\n"));
    }
    Ok(None)
}

fn extract_preceding_line_doc_comments(
    mut node: Node,
    source_code: &str,
) -> Result<Option<String>, LaibraryError> {
    let mut items = Vec::new();

    while node.kind() == "line_comment" {
        if is_doc_comment(&node, DocCommentMarker::Outer) {
            let comment_text = node
                .utf8_text(source_code.as_bytes())
                .map_err(|e| LaibraryError::Parse(e.to_string()))?;
            items.push(comment_text.to_string());
        } else {
            break;
        }

        if let Some(prev) = node.prev_sibling() {
            node = prev;
        } else {
            break;
        }
    }

    Ok(if items.is_empty() {
        None
    } else {
        Some(items.into_iter().rev().collect())
    })
}

pub fn extract_inner_doc_comments(
    node: &Node,
    source_code: &str,
) -> Result<Option<String>, LaibraryError> {
    let mut doc_comment = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "line_comment" {
            if is_doc_comment(&child, DocCommentMarker::Inner) {
                let comment_text = child
                    .utf8_text(source_code.as_bytes())
                    .map_err(|e| LaibraryError::Parse(e.to_string()))?;
                doc_comment.push_str(comment_text);
            } else {
                break;
            }
        } else if !is_block_delimiter(&child) {
            break;
        }
    }
    Ok(if doc_comment.is_empty() {
        None
    } else {
        Some(doc_comment)
    })
}

fn is_block_delimiter(node: &Node) -> bool {
    matches!(node.kind(), "{" | "}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::parsing::test_helpers::make_tree;
    use crate::treesitter_test_helpers::{find_child_node, find_child_nodes};

    mod inner_doc_comments {
        use super::*;

        #[test]
        fn no_doc_comment() {
            let source_code = r#"
pub struct Test {}
"#;
            let tree = make_tree(source_code);

            let result = extract_inner_doc_comments(&tree.root_node(), source_code).unwrap();

            assert!(result.is_none());
        }

        #[test]
        fn single_line_doc_comment() {
            let source_code = r#"
//! This is a file-level doc comment
pub struct Test {}
"#;
            let tree = make_tree(source_code);

            let result = extract_inner_doc_comments(&tree.root_node(), source_code).unwrap();

            assert_eq!(
                result,
                Some("//! This is a file-level doc comment\n".to_string())
            );
        }

        #[test]
        fn multiline_doc_comment() {
            let source_code = r#"
//! This is a file-level doc comment
//! It spans multiple lines

pub struct Test {}
"#;
            let tree = make_tree(source_code);

            let result = extract_inner_doc_comments(&tree.root_node(), source_code).unwrap();

            assert_eq!(
                result,
                Some(
                    "//! This is a file-level doc comment\n//! It spans multiple lines\n"
                        .to_string()
                )
            );
        }

        #[test]
        fn regular_comment_not_doc_comment() {
            let source_code = r#"
// This is a regular comment
pub struct Test {}
"#;
            let tree = make_tree(source_code);

            let result = extract_inner_doc_comments(&tree.root_node(), source_code).unwrap();

            assert!(result.is_none());
        }
    }

    mod outer_doc_comments {
        use super::*;

        #[test]
        fn no_doc_comments() {
            let source_code = r#"
pub struct Test {}
"#;
            let tree = make_tree(source_code);
            let node = find_child_node(tree.root_node(), "struct_item");

            let result = extract_outer_doc_comments(&node, source_code).unwrap();

            assert!(result.is_none());
        }

        #[test]
        fn single_line() {
            let source_code = r#"
/// A documented item
pub struct Test {}
"#;
            let tree = make_tree(source_code);
            let node = find_child_node(tree.root_node(), "struct_item");

            let result = extract_outer_doc_comments(&node, source_code).unwrap();

            assert_eq!(result, Some("/// A documented item\n".to_string()));
        }

        #[test]
        fn multiple_line() {
            let source_code = r#"
/// First line
/// Second line
pub struct Test {}
"#;
            let tree = make_tree(source_code);
            let node = find_child_node(tree.root_node(), "struct_item");

            let result = extract_outer_doc_comments(&node, source_code).unwrap();

            assert_eq!(
                result,
                Some("/// First line\n/// Second line\n".to_string())
            );
        }

        #[test]
        fn inner_doc_comments() {
            let source_code = r#"
//! Inner doc
/// Outer doc
pub struct Test {}
"#;
            let tree = make_tree(source_code);
            let node = find_child_node(tree.root_node(), "struct_item");

            let result = extract_outer_doc_comments(&node, source_code).unwrap();

            assert_eq!(result, Some("/// Outer doc\n".to_string()));
        }

        #[test]
        fn regular_comments() {
            let source_code = r#"
// Regular comment
/// Doc comment
pub struct Test {}
"#;
            let tree = make_tree(source_code);
            let node = find_child_node(tree.root_node(), "struct_item");

            let result = extract_outer_doc_comments(&node, source_code).unwrap();

            assert_eq!(result, Some("/// Doc comment\n".to_string()));
        }

        #[test]
        fn block_doc_comments() {
            let source_code = r#"
/** A block doc comment
 * with multiple lines
 */
pub struct Test {}
"#;
            let tree = make_tree(source_code);
            let node = find_child_node(tree.root_node(), "struct_item");

            let result = extract_outer_doc_comments(&node, source_code).unwrap();

            assert_eq!(
                result,
                Some("/** A block doc comment\n * with multiple lines\n */\n".to_string())
            );
        }

        #[test]
        fn file_level_doc_comments() {
            let source_code = r#"
//! File-level documentation

/// This is the struct's doc
pub struct Test {}
"#;
            let tree = make_tree(source_code);
            let node = find_child_node(tree.root_node(), "struct_item");

            let result = extract_outer_doc_comments(&node, source_code).unwrap();

            assert_eq!(result, Some("/// This is the struct's doc\n".to_string()));
        }

        #[test]
        fn preceding_symbol() {
            let source_code = r#"
/// First struct's doc
pub struct FirstStruct {}

/// Second struct's doc
pub struct SecondStruct {}
"#;
            let tree = make_tree(source_code);
            let nodes = find_child_nodes(tree.root_node(), "struct_item");
            let node = &nodes[1];

            let result = extract_outer_doc_comments(node, source_code).unwrap();

            assert_eq!(result, Some("/// Second struct's doc\n".to_string()));
        }

        #[test]
        fn block_comment_preceded_by_line_comment() {
            let source_code = r#"
/// This line should be ignored
/** This block comment
 * should be returned
 */
pub struct Test {}
"#;
            let tree = make_tree(source_code);
            let node = find_child_node(tree.root_node(), "struct_item");

            let result = extract_outer_doc_comments(&node, source_code).unwrap();

            assert_eq!(
                result,
                Some("/** This block comment\n * should be returned\n */\n".to_string())
            );
        }

        #[test]
        fn line_comment_preceded_by_block_comment() {
            let source_code = r#"
/** Block comment that shouldn't be output */
/// Doc comment that should be output
pub struct Foo {}
"#;
            let tree = make_tree(source_code);
            let node = find_child_node(tree.root_node(), "struct_item");

            let result = extract_outer_doc_comments(&node, source_code).unwrap();

            assert_eq!(
                result,
                Some("/// Doc comment that should be output\n".to_string())
            );
        }

        #[test]
        fn doc_comment_with_attribute() {
            let source_code = r#"
/// The doc comment
#[derive(Debug)]
pub enum Foo {}
"#;
            let tree = make_tree(source_code);
            let node = find_child_node(tree.root_node(), "enum_item");

            let result = extract_outer_doc_comments(&node, source_code).unwrap();

            assert_eq!(result, Some("/// The doc comment\n".to_string()));
        }

        #[test]
        fn doc_comment_with_multiple_attributes() {
            let source_code = r#"
/// The doc comment
#[derive(Debug)]
#[serde(rename = "foo")]
pub enum Foo {}
"#;
            let tree = make_tree(source_code);
            let node = find_child_node(tree.root_node(), "enum_item");

            let result = extract_outer_doc_comments(&node, source_code).unwrap();

            assert_eq!(result, Some("/// The doc comment\n".to_string()));
        }

        #[test]
        fn attribute_without_doc_comment() {
            let source_code = r#"
#[derive(Debug)]
pub enum Foo {}
"#;
            let tree = make_tree(source_code);
            let node = find_child_node(tree.root_node(), "enum_item");

            let result = extract_outer_doc_comments(&node, source_code).unwrap();

            assert!(result.is_none());
        }

        #[test]
        fn trait_method_doc_comments() {
            let source_code = r#"
pub trait TestTrait {
    /// A documented method
    pub fn test_method(&self) -> i32 {
        42
    }
}
"#;
            let tree = make_tree(source_code);
            let trait_node = find_child_node(tree.root_node(), "trait_item");
            let decl_list = find_child_node(trait_node, "declaration_list");
            let method_node = find_child_node(decl_list, "function_item");

            let result = extract_outer_doc_comments(&method_node, source_code).unwrap();

            assert_eq!(result, Some("/// A documented method\n".to_string()));
        }
    }
}
