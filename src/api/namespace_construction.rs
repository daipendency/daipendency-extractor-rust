use super::symbol_resolution::SymbolResolution;
use daipendency_extractor::Namespace;
use std::collections::HashMap;

/// Construct the final namespace hierarchy using the resolved symbols.
pub fn construct_namespaces(
    symbol_resolution: SymbolResolution,
    crate_name: &str,
) -> Vec<Namespace> {
    let mut namespace_by_path: HashMap<String, Namespace> = HashMap::new();
    let crate_name = crate_name.replace("-", "_");

    // Group symbols by namespace
    for resolved_symbol in &symbol_resolution.symbols {
        for module_path in &resolved_symbol.modules {
            let namespace_name = if module_path.is_empty() {
                crate_name.to_string()
            } else {
                format!("{}::{}", crate_name, module_path)
            };
            let namespace = namespace_by_path
                .entry(namespace_name.clone())
                .or_insert_with(|| Namespace {
                    name: namespace_name,
                    symbols: Vec::new(),
                    doc_comment: symbol_resolution.doc_comments.get(module_path).cloned(),
                });
            namespace.symbols.push(resolved_symbol.symbol.clone());
        }
    }

    let mut namespaces: Vec<_> = namespace_by_path.into_values().collect();
    namespaces.sort_by(|a, b| {
        let a_components = a.name.matches("::").count();
        let b_components = b.name.matches("::").count();
        a_components.cmp(&b_components).then(a.name.cmp(&b.name))
    });
    namespaces
}

#[cfg(test)]
mod tests {
    use assertables::assert_contains;

    use super::*;
    use crate::api::symbol_resolution::SymbolDeclaration;
    use crate::test_helpers::{get_namespace, stub_symbol_with_name};

    const STUB_CRATE_NAME: &str = "test_crate";
    const STUB_SYMBOL_NAME: &str = "test";

    #[test]
    fn no_symbols_in_namespace() {
        let namespaces = construct_namespaces(
            SymbolResolution {
                symbols: Vec::new(),
                doc_comments: HashMap::new(),
            },
            STUB_CRATE_NAME,
        );

        assert_eq!(namespaces.len(), 0);
    }

    #[test]
    fn one_symbol_in_namespace() {
        let symbol = stub_symbol_with_name(STUB_SYMBOL_NAME);
        let resolved_symbols = vec![SymbolDeclaration {
            symbol: symbol.clone(),
            modules: vec![String::new()],
        }];

        let namespaces = construct_namespaces(
            SymbolResolution {
                symbols: resolved_symbols,
                doc_comments: HashMap::new(),
            },
            STUB_CRATE_NAME,
        );

        assert_eq!(namespaces.len(), 1);
        let namespace = get_namespace(STUB_CRATE_NAME, &namespaces).unwrap();
        let namespace_symbol = namespace.get_symbol(STUB_SYMBOL_NAME).unwrap();
        assert_eq!(namespace_symbol, &symbol);
    }

    #[test]
    fn multiple_symbols_in_namespace() {
        let module_name = String::new();
        let symbol1 = stub_symbol_with_name("first_symbol");
        let symbol2 = stub_symbol_with_name("second_symbol");
        let resolved_symbols = vec![
            SymbolDeclaration {
                symbol: symbol1.clone(),
                modules: vec![module_name.clone()],
            },
            SymbolDeclaration {
                symbol: symbol2.clone(),
                modules: vec![module_name.clone()],
            },
        ];

        let namespaces = construct_namespaces(
            SymbolResolution {
                symbols: resolved_symbols,
                doc_comments: HashMap::new(),
            },
            STUB_CRATE_NAME,
        );

        assert_eq!(namespaces.len(), 1);
        let root = get_namespace(STUB_CRATE_NAME, &namespaces).unwrap();
        assert_eq!(root.symbols.len(), 2);
        assert_contains!(root.symbols, &symbol1);
        assert_contains!(root.symbols, &symbol2);
    }

    #[test]
    fn different_symbols_across_namespaces() {
        let symbol1 = stub_symbol_with_name(&format!("{}_root", STUB_SYMBOL_NAME));
        let symbol2 = stub_symbol_with_name(&format!("{}_nested", STUB_SYMBOL_NAME));
        let resolved_symbols = vec![
            SymbolDeclaration {
                symbol: symbol1.clone(),
                modules: vec![String::new()],
            },
            SymbolDeclaration {
                symbol: symbol2.clone(),
                modules: vec!["submodule".to_string()],
            },
        ];

        let namespaces = construct_namespaces(
            SymbolResolution {
                symbols: resolved_symbols,
                doc_comments: HashMap::new(),
            },
            STUB_CRATE_NAME,
        );
        assert_eq!(namespaces.len(), 2);

        let root = get_namespace(STUB_CRATE_NAME, &namespaces).unwrap();
        assert_eq!(root.symbols, vec![symbol1]);

        let nested =
            get_namespace(&format!("{}::submodule", STUB_CRATE_NAME), &namespaces).unwrap();
        assert_eq!(nested.symbols, vec![symbol2]);
    }

    #[test]
    fn same_symbol_across_namespaces() {
        let symbol = stub_symbol_with_name(STUB_SYMBOL_NAME);
        let resolved_symbols = vec![SymbolDeclaration {
            symbol: symbol.clone(),
            modules: vec!["outer".to_string(), "outer::inner".to_string()],
        }];

        let namespaces = construct_namespaces(
            SymbolResolution {
                symbols: resolved_symbols,
                doc_comments: HashMap::new(),
            },
            STUB_CRATE_NAME,
        );

        assert_eq!(namespaces.len(), 2);
        let outer_namespace =
            get_namespace(&format!("{}::outer", STUB_CRATE_NAME), &namespaces).unwrap();
        let inner_namespace =
            get_namespace(&format!("{}::outer::inner", STUB_CRATE_NAME), &namespaces).unwrap();
        assert_eq!(outer_namespace.symbols, vec![symbol.clone()]);
        assert_eq!(inner_namespace.symbols, vec![symbol]);
    }

    #[test]
    fn hypenated_crate_name() {
        let crate_name = "test-crate";
        let symbol = stub_symbol_with_name(STUB_SYMBOL_NAME);
        let resolved_symbols = vec![SymbolDeclaration {
            symbol: symbol.clone(),
            modules: vec![String::new()],
        }];

        let namespaces = construct_namespaces(
            SymbolResolution {
                symbols: resolved_symbols,
                doc_comments: HashMap::new(),
            },
            crate_name,
        );

        assert_eq!(namespaces.len(), 1);
        let final_crate_name = crate_name.replace("-", "_");
        let root = get_namespace(&final_crate_name, &namespaces).unwrap();
        assert_eq!(root.name, final_crate_name);
    }

    #[test]
    fn doc_comment() {
        let doc_comment = "This is a stub doc comment";
        let resolved_symbols = vec![SymbolDeclaration {
            symbol: stub_symbol_with_name(STUB_SYMBOL_NAME),
            modules: vec![String::new()],
        }];

        let namespaces = construct_namespaces(
            SymbolResolution {
                symbols: resolved_symbols,
                doc_comments: HashMap::from([(String::new(), doc_comment.to_string())]),
            },
            STUB_CRATE_NAME,
        );

        assert_eq!(namespaces.len(), 1);
        let root = get_namespace(STUB_CRATE_NAME, &namespaces).unwrap();
        assert_eq!(root.doc_comment, Some(doc_comment.to_string()));
    }

    mod sorting {
        use super::*;

        #[test]
        fn children() {
            let resolved_symbols = vec![
                SymbolDeclaration {
                    symbol: stub_symbol_with_name("test1"),
                    modules: vec![String::new()],
                },
                SymbolDeclaration {
                    symbol: stub_symbol_with_name("test2"),
                    modules: vec!["error".to_string()],
                },
            ];

            let namespaces = construct_namespaces(
                SymbolResolution {
                    symbols: resolved_symbols,
                    doc_comments: HashMap::new(),
                },
                STUB_CRATE_NAME,
            );

            let names: Vec<_> = namespaces.iter().map(|n| n.name.as_str()).collect();
            assert_eq!(names, vec!["test_crate", "test_crate::error",]);
        }

        #[test]
        fn siblings() {
            let resolved_symbols = vec![
                SymbolDeclaration {
                    symbol: stub_symbol_with_name("test1"),
                    modules: vec!["submodule1".to_string()],
                },
                SymbolDeclaration {
                    symbol: stub_symbol_with_name("test2"),
                    modules: vec!["submodule".to_string()],
                },
            ];

            let namespaces = construct_namespaces(
                SymbolResolution {
                    symbols: resolved_symbols,
                    doc_comments: HashMap::new(),
                },
                STUB_CRATE_NAME,
            );

            let names: Vec<_> = namespaces.iter().map(|n| n.name.as_str()).collect();
            assert_eq!(
                names,
                vec!["test_crate::submodule", "test_crate::submodule1",]
            );
        }
    }
}
