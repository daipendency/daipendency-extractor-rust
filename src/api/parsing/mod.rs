use daipendency_extractor::ExtractionError;
use daipendency_extractor::Symbol;
use tree_sitter::{Node, Parser};

mod doc_comments;
mod files;
mod helpers;
mod macros;
mod symbol_reexports;
mod symbols;
mod test_helpers;

use doc_comments::extract_inner_doc_comments;
use helpers::{extract_name, get_declaration_list, is_public};
use macros::get_macro_source_code;
use symbol_reexports::extract_symbol_reexports;
use symbols::get_symbol_source_code;

pub use files::{RustFile, RustSymbol};

pub fn parse_rust_file(content: &str, parser: &mut Parser) -> Result<RustFile, ExtractionError> {
    let tree = parser
        .parse(content, None)
        .ok_or_else(|| ExtractionError::Malformed("Failed to parse source file".to_string()))?;

    let doc_comment = extract_inner_doc_comments(&tree.root_node(), content)?;
    let symbols = extract_symbols_from_module(tree.root_node(), content)?;
    Ok(RustFile {
        doc_comment,
        symbols,
    })
}

fn extract_symbols_from_module(
    module_node: Node,
    source_code: &str,
) -> Result<Vec<RustSymbol>, ExtractionError> {
    let mut symbols = Vec::new();
    let mut cursor = module_node.walk();

    for child in module_node.children(&mut cursor) {
        match child.kind() {
            "function_item" | "struct_item" | "enum_item" | "trait_item" => {
                if !is_public(&child) {
                    continue;
                }
                let name = extract_name(&child, source_code)?;
                symbols.push(RustSymbol::Symbol {
                    symbol: Symbol {
                        name,
                        source_code: get_symbol_source_code(child, source_code)?,
                    },
                });
            }
            "macro_definition" => {
                let source_code_opt = get_macro_source_code(child, source_code)?;
                if let Some(macro_source_code) = source_code_opt {
                    let name = extract_name(&child, source_code)?;
                    symbols.push(RustSymbol::Symbol {
                        symbol: Symbol {
                            name,
                            source_code: macro_source_code,
                        },
                    });
                }
            }
            "use_declaration" => {
                symbols.extend(extract_symbol_reexports(&child, source_code)?);
            }
            "mod_item" => {
                let inner_mod_name = extract_name(&child, source_code)?;
                let is_public = is_public(&child);

                if let Some(declaration_list) = get_declaration_list(child) {
                    // This is a module block (`mod foo { ... }`)
                    if is_public {
                        let doc_comment =
                            extract_inner_doc_comments(&declaration_list, source_code)?;
                        let inner_mod_symbols =
                            extract_symbols_from_module(declaration_list, source_code)?;
                        symbols.push(RustSymbol::Module {
                            name: inner_mod_name,
                            content: inner_mod_symbols,
                            doc_comment,
                        });
                    }
                } else {
                    // This is a module declaration (`mod foo;`)
                    symbols.push(RustSymbol::ModuleImport {
                        name: inner_mod_name,
                        is_reexported: is_public,
                    });
                }
            }
            _ => {}
        }
    }

    Ok(symbols)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::setup_parser;

    #[test]
    fn empty_source_file() {
        let source_code = "";
        let mut parser = setup_parser();

        let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

        assert!(rust_file.symbols.is_empty());
    }

    #[test]
    fn invalid_syntax() {
        let source_code = "echo 'Hello, World!'";
        let mut parser = setup_parser();

        let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

        assert!(rust_file.symbols.is_empty());
    }

    #[test]
    fn reexports_multiple_symbols() {
        let source_code = r#"
pub use other::{One, Two};
"#;
        let mut parser = setup_parser();

        let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

        assert!(rust_file.get_symbol("One").is_some());
        assert!(rust_file.get_symbol("Two").is_some());
    }

    #[test]
    fn function_declaration() {
        let source_code = r#"
pub fn test_function() -> i32 {
    return 42;
}
"#;
        let mut parser = setup_parser();

        let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

        let symbol = rust_file.get_symbol("test_function").unwrap();
        let RustSymbol::Symbol { symbol } = symbol else {
            panic!("Expected a symbol")
        };
        assert_eq!(symbol.source_code, "pub fn test_function() -> i32;");
    }

    #[test]
    fn macro_declaration() {
        let source_code = r#"
#[macro_export]
macro_rules! test_macro {
    () => { println!("Hello, world!"); }
}
"#;
        let mut parser = setup_parser();

        let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

        let symbol = rust_file.get_symbol("test_macro").unwrap();
        let RustSymbol::Symbol { symbol } = symbol else {
            panic!("Expected a symbol")
        };
        assert_eq!(
            symbol.source_code,
            "#[macro_export]\nmacro_rules! test_macro;"
        );
    }

    #[test]
    fn private_symbols() {
        let source_code = r#"
fn private_function() {}
"#;
        let mut parser = setup_parser();

        let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

        assert_eq!(rust_file.symbols.len(), 0);
    }

    mod inner_modules {
        use super::*;

        #[test]
        fn public_modules() {
            let source_code = r#"
pub mod inner {
    pub fn nested_function() -> String {}
}
"#;
            let mut parser = setup_parser();

            let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

            let symbol = rust_file.get_symbol("inner::nested_function").unwrap();
            assert!(matches!(symbol, RustSymbol::Symbol { .. }));
        }

        #[test]
        fn private_modules() {
            let source_code = r#"
mod private {
    pub fn private_function() -> String {}
}
"#;
            let mut parser = setup_parser();

            let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

            assert!(
                rust_file.symbols.is_empty(),
                "Private modules should be ignored"
            );
        }

        #[test]
        fn empty_modules() {
            let source_code = r#"
pub mod empty {}
"#;
            let mut parser = setup_parser();

            let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

            let empty_content = rust_file.get_module("empty").unwrap();
            assert_eq!(rust_file.symbols.len(), 1);
            assert!(empty_content.is_empty());
        }

        #[test]
        fn inner_module_symbols() {
            let source_code = r#"
pub mod inner {
    pub mod deeper {
        pub enum DeeperEnum {
            A, B
        }
    }
}
"#;
            let mut parser = setup_parser();

            let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

            let deeper_enum = rust_file.get_symbol("inner::deeper::DeeperEnum").unwrap();
            assert!(matches!(deeper_enum, RustSymbol::Symbol { .. }));
        }

        #[test]
        fn module_declarations() {
            let source_code = r#"
pub mod other;
"#;
            let mut parser = setup_parser();

            let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

            let module_declaration = rust_file.get_symbol("other").unwrap();
            assert!(matches!(
                module_declaration,
                RustSymbol::ModuleImport { .. }
            ));
        }
    }

    mod doc_comments {
        use super::*;

        #[test]
        fn file_without_docs() {
            let source_code = r#"
pub struct Test {}
"#;
            let mut parser = setup_parser();

            let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

            assert!(rust_file.doc_comment.is_none());
        }

        #[test]
        fn file_with_docs() {
            let source_code = r#"
//! File-level documentation
pub struct Test {}
"#;
            let mut parser = setup_parser();

            let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

            assert_eq!(
                rust_file.doc_comment,
                Some("//! File-level documentation\n".to_string())
            );
        }

        #[test]
        fn symbol_with_outer_doc_comment() {
            let source_code = r#"
/// Symbol documentation
pub struct Test {}
"#;
            let mut parser = setup_parser();

            let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

            let symbol = rust_file.get_symbol("Test").unwrap();
            let RustSymbol::Symbol { symbol } = symbol else {
                panic!("Expected a symbol")
            };
            assert_eq!(
                symbol.source_code,
                "/// Symbol documentation\npub struct Test {}"
            );
        }

        #[test]
        fn file_and_symbol_with_doc_comments() {
            let source_code = r#"
//! File-level documentation
/// Symbol documentation
pub struct Test {}
"#;
            let mut parser = setup_parser();

            let rust_file = parse_rust_file(source_code, &mut parser).unwrap();

            assert_eq!(
                rust_file.doc_comment,
                Some("//! File-level documentation\n".to_string())
            );
            let symbol = rust_file.get_symbol("Test").unwrap();
            let RustSymbol::Symbol { symbol } = symbol else {
                panic!("Expected a symbol")
            };
            assert_eq!(
                symbol.source_code,
                "/// Symbol documentation\npub struct Test {}"
            );
        }
    }
}
