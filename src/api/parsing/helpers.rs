use daipendency_extractor::ExtractionError;
use tree_sitter::Node;

pub fn is_public(node: &Node) -> bool {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    children
        .iter()
        .any(|child| child.kind() == "visibility_modifier")
}

pub fn get_declaration_list(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    children
        .into_iter()
        .find(|n| n.kind() == "declaration_list")
}

pub fn extract_attributes(node: &Node, source_code: &str) -> Result<Vec<String>, ExtractionError> {
    let mut current = node.prev_sibling();
    let mut items = Vec::new();

    while let Some(sibling) = current {
        if sibling.kind() != "attribute_item" {
            break;
        }

        let text = sibling
            .utf8_text(source_code.as_bytes())
            .map_err(|e| ExtractionError::Parse(e.to_string()))?;
        items.push(text.to_string());

        current = sibling.prev_sibling();
    }

    items.reverse();
    Ok(items)
}

pub fn extract_name(node: &Node, source_code: &str) -> Result<String, ExtractionError> {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    children
        .iter()
        .find(|child| matches!(child.kind(), "identifier" | "type_identifier"))
        .and_then(|child| {
            child
                .utf8_text(source_code.as_bytes())
                .map(|s| s.to_string())
                .ok()
        })
        .ok_or_else(|| ExtractionError::Parse("Failed to extract name".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::parsing::test_helpers::make_tree;
    use crate::treesitter_test_helpers::find_child_node;

    mod is_public {
        use super::*;

        #[test]
        fn public_function() {
            let tree = make_tree("pub fn test() {}");
            let function = find_child_node(tree.root_node(), "function_item");

            assert!(is_public(&function));
        }

        #[test]
        fn private_function() {
            let tree = make_tree("fn test() {}");
            let function = find_child_node(tree.root_node(), "function_item");

            assert!(!is_public(&function));
        }

        #[test]
        fn public_crate_function() {
            let tree = make_tree("pub(crate) fn test() {}");
            let function = find_child_node(tree.root_node(), "function_item");

            assert!(is_public(&function));
        }

        #[test]
        fn public_super_function() {
            let tree = make_tree("pub(super) fn test() {}");
            let function = find_child_node(tree.root_node(), "function_item");

            assert!(is_public(&function));
        }
    }

    mod extract_attributes {
        use super::*;

        #[test]
        fn no_attributes() {
            let tree = make_tree("fn test() {}");
            let function = find_child_node(tree.root_node(), "function_item");

            let attributes = extract_attributes(&function, "fn test() {}").unwrap();

            assert!(attributes.is_empty());
        }

        #[test]
        fn single_attribute() {
            let source = "#[derive(Debug)]\nfn test() {}";
            let tree = make_tree(source);
            let function = find_child_node(tree.root_node(), "function_item");

            let attributes = extract_attributes(&function, source).unwrap();

            assert_eq!(attributes, vec!["#[derive(Debug)]"]);
        }

        #[test]
        fn multiple_attributes() {
            let source = "#[derive(Debug)]\n#[cfg(test)]\nfn test() {}";
            let tree = make_tree(source);
            let function = find_child_node(tree.root_node(), "function_item");

            let attributes = extract_attributes(&function, source).unwrap();

            assert_eq!(attributes, vec!["#[derive(Debug)]", "#[cfg(test)]"]);
        }

        #[test]
        fn attributes_with_complex_content() {
            let source =
                "#[cfg_attr(feature = \"serde\", derive(Serialize, Deserialize))]\nfn test() {}";
            let tree = make_tree(source);
            let function = find_child_node(tree.root_node(), "function_item");

            let attributes = extract_attributes(&function, source).unwrap();

            assert_eq!(
                attributes,
                vec!["#[cfg_attr(feature = \"serde\", derive(Serialize, Deserialize))]"]
            );
        }
    }

    mod extract_name {
        use super::*;

        #[test]
        fn function_name() {
            let tree = make_tree("fn test_function() {}");
            let function = find_child_node(tree.root_node(), "function_item");

            let name = extract_name(&function, "fn test_function() {}").unwrap();

            assert_eq!(name, "test_function");
        }

        #[test]
        fn struct_name() {
            let tree = make_tree("struct TestStruct {}");
            let struct_node = find_child_node(tree.root_node(), "struct_item");

            let name = extract_name(&struct_node, "struct TestStruct {}").unwrap();

            assert_eq!(name, "TestStruct");
        }

        #[test]
        fn enum_name() {
            let tree = make_tree("enum TestEnum { A, B }");
            let enum_node = find_child_node(tree.root_node(), "enum_item");

            let name = extract_name(&enum_node, "enum TestEnum { A, B }").unwrap();

            assert_eq!(name, "TestEnum");
        }

        #[test]
        fn trait_name() {
            let tree = make_tree("trait TestTrait {}");
            let trait_node = find_child_node(tree.root_node(), "trait_item");

            let name = extract_name(&trait_node, "trait TestTrait {}").unwrap();

            assert_eq!(name, "TestTrait");
        }

        #[test]
        fn module_name() {
            let tree = make_tree("mod test_module {}");
            let module_node = find_child_node(tree.root_node(), "mod_item");

            let name = extract_name(&module_node, "mod test_module {}").unwrap();

            assert_eq!(name, "test_module");
        }

        #[test]
        fn should_fail_on_invalid_node() {
            let tree = make_tree("// just a comment");
            let comment = find_child_node(tree.root_node(), "line_comment");

            let result = extract_name(&comment, "// just a comment");

            assert!(result.is_err());
        }
    }
}
