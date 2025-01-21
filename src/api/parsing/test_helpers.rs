#![cfg(test)]

use crate::test_helpers::setup_parser;
use tree_sitter::{Node, Tree};

pub struct TestTree {
    tree: Tree,
}

impl TestTree {
    pub fn root_node(&self) -> Node {
        self.tree.root_node()
    }
}

pub fn make_tree(source_code: &str) -> TestTree {
    TestTree {
        tree: setup_parser().parse(source_code, None).unwrap(),
    }
}
