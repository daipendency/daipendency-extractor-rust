#![cfg(test)]

use crate::extractor::RustExtractor;
use daipendency_extractor::{Extractor, Namespace, Symbol};
use tree_sitter::Parser;

pub fn setup_parser() -> Parser {
    let mut parser = Parser::new();
    let analyser = RustExtractor;
    parser
        .set_language(&analyser.get_parser_language())
        .unwrap();
    parser
}

pub fn get_namespace<'a>(name: &str, namespaces: &'a [Namespace]) -> Option<&'a Namespace> {
    namespaces.iter().find(|n| n.name == name)
}

pub fn stub_symbol_with_name(name: &str) -> Symbol {
    Symbol {
        name: name.to_string(),
        source_code: format!("pub fn {}() {{}}", name).to_string(),
    }
}

pub fn stub_symbol() -> Symbol {
    let name = "test";
    Symbol {
        name: name.to_string(),
        source_code: format!("pub fn {}() {{}}", name).to_string(),
    }
}
