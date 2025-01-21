#![cfg(test)]

use tree_sitter::Node;

pub fn find_child_nodes<'tree>(root: Node<'tree>, kind: &str) -> Vec<Node<'tree>> {
    let mut cursor = root.walk();
    root.children(&mut cursor)
        .filter(|node| node.kind() == kind)
        .collect()
}

pub fn find_child_node<'tree>(root: Node<'tree>, kind: &str) -> Node<'tree> {
    let nodes = find_child_nodes(root, kind);
    assert_eq!(nodes.len(), 1);
    nodes[0]
}
