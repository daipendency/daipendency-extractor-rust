use daipendency_extractor::LaibraryError;
use daipendency_extractor::Symbol;
use std::collections::{HashMap, HashSet};

use super::symbol_collection::{Module, Reference};

#[derive(Debug, Clone)]
pub struct ResolvedSymbol {
    pub symbol: Symbol,
    pub modules: Vec<String>,
}

#[derive(Debug)]
pub struct SymbolResolution {
    pub symbols: Vec<ResolvedSymbol>,
    pub doc_comments: HashMap<String, String>,
}

/// Resolve symbol references by matching them with their corresponding definitions.
pub fn resolve_symbols(all_modules: &[Module]) -> Result<SymbolResolution, LaibraryError> {
    let public_modules: Vec<Module> = all_modules
        .iter()
        .filter(|m| m.name.is_empty() || m.is_public)
        .cloned()
        .collect();

    let public_symbols = match resolve_public_symbols(all_modules, &public_modules) {
        Ok(value) => value,
        Err(err) => return Err(err),
    };

    let doc_comments = get_doc_comments_by_module(&public_modules);

    Ok(SymbolResolution {
        symbols: public_symbols,
        doc_comments,
    })
}

fn resolve_public_symbols(
    all_modules: &[Module],
    public_modules: &[Module],
) -> Result<Vec<ResolvedSymbol>, LaibraryError> {
    let mut resolved_symbols: HashMap<String, ResolvedSymbol> = HashMap::new();
    let mut references_by_symbol_path: HashMap<String, Vec<String>> = HashMap::new();

    // Collect all symbol definitions and references
    for module in all_modules {
        for symbol in &module.definitions {
            let symbol_path = get_symbol_path(&symbol.name, module);
            resolved_symbols.insert(
                symbol_path.clone(),
                ResolvedSymbol {
                    symbol: symbol.clone(),
                    modules: vec![module.name.clone()],
                },
            );
        }

        for reference in &module.references {
            match reference {
                Reference::Symbol(source_path) => {
                    let normalised_path = normalise_reference(source_path, &module.name)?;
                    references_by_symbol_path
                        .entry(normalised_path)
                        .or_default()
                        .push(module.name.clone());
                }
                Reference::Wildcard(module_path) => {
                    let normalised_path = normalise_reference(module_path, &module.name)?;
                    let referenced_module = all_modules
                        .iter()
                        .find(|m| m.name == normalised_path)
                        .ok_or_else(|| {
                            LaibraryError::Parse(format!(
                                "Could not find module '{}'",
                                normalised_path
                            ))
                        })?;

                    referenced_module
                        .definitions
                        .iter()
                        .map(|symbol| {
                            let symbol_path = get_symbol_path(&symbol.name, referenced_module);
                            references_by_symbol_path
                                .entry(symbol_path)
                                .or_default()
                                .push(module.name.clone());
                        })
                        .for_each(drop);
                }
            }
        }
    }

    // Resolve reexports in public modules
    let public_module_paths: HashSet<String> =
        public_modules.iter().map(|m| m.name.clone()).collect();
    for (source_path, referencing_modules) in &references_by_symbol_path {
        if let Some(resolved) = resolved_symbols.get_mut(source_path) {
            let mut new_modules = resolved.modules.clone();
            new_modules.extend(referencing_modules.iter().cloned());
            let new_modules_set: HashSet<_> = new_modules.into_iter().collect();
            resolved.modules = new_modules_set
                .intersection(&public_module_paths)
                .cloned()
                .collect();
        } else {
            // The symbol can't be found in the codebase, so it's likely to be a dependency
            let symbol_name = source_path.split("::").last().unwrap();
            let resolved_symbol = ResolvedSymbol {
                symbol: Symbol {
                    name: symbol_name.to_string(),
                    source_code: format!("pub use {};", source_path),
                },
                modules: referencing_modules.clone(),
            };
            resolved_symbols.insert(source_path.to_string(), resolved_symbol);
        }
    }

    let public_symbols: Vec<ResolvedSymbol> = resolved_symbols
        .into_values()
        .filter(|symbol| {
            let symbol_modules: HashSet<_> = symbol.modules.iter().cloned().collect();
            symbol_modules
                .intersection(&public_module_paths)
                .next()
                .is_some()
        })
        .collect();

    Ok(public_symbols)
}

fn get_symbol_path(symbol_name: &str, module: &Module) -> String {
    if module.name.is_empty() {
        symbol_name.to_string()
    } else {
        format!("{}::{}", module.name, symbol_name)
    }
}

fn normalise_reference(reference: &str, current_module: &str) -> Result<String, LaibraryError> {
    if let Some(stripped) = reference.strip_prefix("crate::") {
        Ok(stripped.to_string())
    } else if let Some(stripped) = reference.strip_prefix("super::") {
        if current_module.is_empty() {
            return Err(LaibraryError::Parse(
                "Cannot use super from the root module".to_string(),
            ));
        }
        if let Some(parent) = current_module.rfind("::") {
            Ok(format!("{}::{}", &current_module[..parent], stripped))
        } else {
            Ok(stripped.to_string())
        }
    } else if let Some(stripped) = reference.strip_prefix("self::") {
        if current_module.is_empty() {
            Ok(stripped.to_string())
        } else {
            Ok(format!("{}::{}", current_module, stripped))
        }
    } else {
        Ok(reference.to_string())
    }
}

fn get_doc_comments_by_module(public_modules: &[Module]) -> HashMap<String, String> {
    let doc_comments = public_modules
        .iter()
        .filter_map(|module| {
            module
                .doc_comment
                .as_ref()
                .map(|doc| (module.name.clone(), doc.clone()))
        })
        .collect();
    doc_comments
}

#[cfg(test)]
mod tests {
    use super::*;
    use assertables::*;

    impl SymbolResolution {
        fn get_symbol_modules(&self, symbol: Symbol) -> Vec<String> {
            self.symbols
                .iter()
                .find(|s| s.symbol == symbol)
                .expect("No matching symbol found")
                .modules
                .clone()
        }
    }

    mod symbol_definitions {
        use super::*;
        use crate::test_helpers::stub_symbol;

        #[test]
        fn at_root() {
            let symbol = stub_symbol();
            let modules = vec![Module {
                name: String::new(),
                definitions: vec![symbol.clone()],
                references: Vec::new(),
                is_public: true,
                doc_comment: None,
            }];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(resolution.get_symbol_modules(symbol), vec![String::new()]);
        }

        #[test]
        fn at_submodule() {
            let symbol = stub_symbol();
            let modules = vec![Module {
                name: "outer::inner".to_string(),
                definitions: vec![symbol.clone()],
                references: Vec::new(),
                is_public: true,
                doc_comment: None,
            }];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(
                resolution.get_symbol_modules(symbol),
                vec!["outer::inner".to_string()]
            );
        }
    }

    mod reexports {
        use crate::test_helpers::{stub_symbol, stub_symbol_with_name};

        use super::*;

        #[test]
        fn via_public_module() {
            let symbol = stub_symbol();
            let modules = vec![
                Module {
                    name: String::new(),
                    definitions: Vec::new(),
                    references: vec![Reference::Symbol("inner::test".to_string())],
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "inner".to_string(),
                    definitions: vec![symbol.clone()],
                    references: Vec::new(),
                    is_public: true,
                    doc_comment: None,
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(
                resolution.get_symbol_modules(symbol),
                vec![String::new(), "inner".to_string()]
            );
        }

        #[test]
        fn via_private_module() {
            let symbol = stub_symbol();
            let modules = vec![
                Module {
                    name: String::new(),
                    definitions: Vec::new(),
                    references: vec![Reference::Symbol("inner::test".to_string())],
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "inner".to_string(),
                    definitions: vec![symbol.clone()],
                    references: Vec::new(),
                    is_public: false,
                    doc_comment: None,
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(resolution.get_symbol_modules(symbol), vec![String::new()]);
        }

        #[test]
        fn via_nested_public_module() {
            let symbol = stub_symbol();
            let modules = vec![
                Module {
                    name: "foo::bar".to_string(),
                    definitions: Vec::new(),
                    references: vec![Reference::Symbol("outer::inner::test".to_string())],
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "outer::inner".to_string(),
                    definitions: vec![symbol.clone()],
                    references: Vec::new(),
                    is_public: true,
                    doc_comment: None,
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(
                resolution.get_symbol_modules(symbol),
                vec!["foo::bar".to_string(), "outer::inner".to_string()]
            );
        }

        #[test]
        fn partial_private_module_reexport() {
            let reexported_symbol = stub_symbol_with_name("reexported");
            let non_reexported_symbol = stub_symbol_with_name("non_reexported");
            let modules = vec![
                Module {
                    name: String::new(),
                    definitions: Vec::new(),
                    references: vec![Reference::Symbol(format!(
                        "inner::{}",
                        reexported_symbol.name
                    ))],
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "inner".to_string(),
                    definitions: vec![reexported_symbol.clone(), non_reexported_symbol.clone()],
                    references: Vec::new(),
                    is_public: false,
                    doc_comment: None,
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(
                resolution.get_symbol_modules(reexported_symbol),
                vec![String::new()]
            );
        }

        #[test]
        fn missing_reference() {
            let reference_source_code = "missing::test";
            let modules = vec![Module {
                name: "outer".to_string(),
                definitions: Vec::new(),
                references: vec![Reference::Symbol(reference_source_code.to_string())],
                is_public: true,
                doc_comment: None,
            }];

            let result = resolve_symbols(&modules).unwrap();

            assert_eq!(result.symbols.len(), 1);
            let resolved_symbol = result.symbols[0].clone();
            assert_eq!(
                resolved_symbol.symbol.source_code,
                format!("pub use {};", reference_source_code)
            );
            assert_set_eq!(resolved_symbol.modules, vec!["outer".to_string()]);
        }

        #[test]
        fn clashing_reexports() {
            let foo_symbol = stub_symbol_with_name("test");
            let bar_symbol = Symbol {
                name: "test".to_string(),
                source_code: "pub fn test() -> i32;".to_string(),
            };
            let modules = vec![
                Module {
                    name: "foo".to_string(),
                    definitions: vec![foo_symbol.clone()],
                    references: Vec::new(),
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "bar".to_string(),
                    definitions: vec![bar_symbol.clone()],
                    references: Vec::new(),
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "reexporter1".to_string(),
                    definitions: Vec::new(),
                    references: vec![Reference::Symbol("foo::test".to_string())],
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "reexporter2".to_string(),
                    definitions: Vec::new(),
                    references: vec![Reference::Symbol("bar::test".to_string())],
                    is_public: true,
                    doc_comment: None,
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 2);
            assert_set_eq!(
                resolution.get_symbol_modules(foo_symbol),
                vec!["foo".to_string(), "reexporter1".to_string()]
            );
            assert_set_eq!(
                resolution.get_symbol_modules(bar_symbol),
                vec!["bar".to_string(), "reexporter2".to_string()],
            );
        }

        #[test]
        fn crate_path_reference() {
            let symbol = stub_symbol();
            let modules = vec![
                Module {
                    name: String::new(),
                    definitions: Vec::new(),
                    references: vec![Reference::Symbol("crate::inner::test".to_string())],
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "inner".to_string(),
                    definitions: vec![symbol.clone()],
                    references: Vec::new(),
                    is_public: false,
                    doc_comment: None,
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(resolution.get_symbol_modules(symbol), vec![String::new()]);
        }

        #[test]
        fn super_path_from_root() {
            let modules = vec![Module {
                name: String::new(),
                definitions: Vec::new(),
                references: vec![Reference::Symbol("super::test".to_string())],
                is_public: true,
                doc_comment: None,
            }];

            let result = resolve_symbols(&modules);

            assert!(matches!(
                result,
                Err(LaibraryError::Parse(msg)) if msg == "Cannot use super from the root module"
            ));
        }

        #[test]
        fn super_path_from_child() {
            let symbol = stub_symbol();
            let modules = vec![
                Module {
                    name: "".to_string(),
                    definitions: vec![symbol.clone()],
                    references: Vec::new(),
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "child".to_string(),
                    definitions: Vec::new(),
                    references: vec![Reference::Symbol("super::test".to_string())],
                    is_public: false,
                    doc_comment: None,
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(resolution.get_symbol_modules(symbol), vec!["".to_string()]);
        }

        #[test]
        fn super_path_from_grandchild() {
            let symbol = stub_symbol();
            let modules = vec![
                Module {
                    name: "parent".to_string(),
                    definitions: vec![symbol.clone()],
                    references: Vec::new(),
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "parent::child".to_string(),
                    definitions: Vec::new(),
                    references: vec![Reference::Symbol("super::test".to_string())],
                    is_public: false,
                    doc_comment: None,
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(
                resolution.get_symbol_modules(symbol),
                vec!["parent".to_string()]
            );
        }

        #[test]
        fn self_path_from_root() {
            let symbol = stub_symbol();
            let modules = vec![
                Module {
                    name: "".to_string(),
                    definitions: Vec::new(),
                    references: vec![Reference::Symbol("self::child::test".to_string())],
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "child".to_string(),
                    definitions: vec![symbol.clone()],
                    references: Vec::new(),
                    is_public: false,
                    doc_comment: None,
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(resolution.get_symbol_modules(symbol), vec!["".to_string()]);
        }

        #[test]
        fn self_path_from_child() {
            let symbol = stub_symbol();
            let modules = vec![
                Module {
                    name: "module".to_string(),
                    definitions: Vec::new(),
                    references: vec![Reference::Symbol("self::inner::test".to_string())],
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "module::inner".to_string(),
                    definitions: vec![symbol.clone()],
                    references: Vec::new(),
                    is_public: false,
                    doc_comment: None,
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(
                resolution.get_symbol_modules(symbol),
                vec!["module".to_string()]
            );
        }

        #[test]
        fn wildcard_reexport() {
            let symbol1 = stub_symbol_with_name("one");
            let symbol2 = stub_symbol_with_name("two");
            let modules = vec![
                Module {
                    name: String::new(),
                    definitions: Vec::new(),
                    references: vec![Reference::Wildcard("inner".to_string())],
                    is_public: true,
                    doc_comment: None,
                },
                Module {
                    name: "inner".to_string(),
                    definitions: vec![symbol1.clone(), symbol2.clone()],
                    references: Vec::new(),
                    is_public: false,
                    doc_comment: None,
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 2);
            assert_set_eq!(resolution.get_symbol_modules(symbol1), vec![String::new()]);
            assert_set_eq!(resolution.get_symbol_modules(symbol2), vec![String::new()]);
        }
    }

    mod doc_comments {
        use super::*;

        #[test]
        fn namespace_without_doc_comment() {
            let modules = vec![Module {
                name: "text".to_string(),
                definitions: vec![],
                references: vec![],
                is_public: true,
                doc_comment: None,
            }];

            let resolution = resolve_symbols(&modules).unwrap();

            assert!(resolution.doc_comments.is_empty());
        }

        #[test]
        fn namespace_with_doc_comment() {
            let modules = vec![Module {
                name: "text".to_string(),
                definitions: vec![],
                references: vec![],
                is_public: true,
                doc_comment: Some("Module for text processing".to_string()),
            }];

            let resolution = resolve_symbols(&modules).unwrap();
            assert_eq!(resolution.doc_comments.len(), 1);
            assert_eq!(
                resolution.doc_comments.get("text"),
                Some(&"Module for text processing".to_string())
            );
        }
    }
}
