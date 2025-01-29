use daipendency_extractor::ExtractionError;
use daipendency_extractor::Symbol;
use std::collections::{HashMap, HashSet};

use super::module_directory::{Module, ModuleItem};
use super::parsing::ImportType;

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

#[derive(Debug)]
struct SymbolReference {
    source_path: String,
    referencing_module: String,
    import_type: ImportType,
}

/// Resolve symbol references by matching them with their corresponding definitions.
pub fn resolve_symbols(all_modules: &[Module]) -> Result<SymbolResolution, ExtractionError> {
    let public_symbols = resolve_public_symbols(all_modules)?;

    let doc_comments = get_doc_comments_by_module(all_modules);

    Ok(SymbolResolution {
        symbols: public_symbols,
        doc_comments,
    })
}

fn resolve_public_symbols(all_modules: &[Module]) -> Result<Vec<ResolvedSymbol>, ExtractionError> {
    let (mut resolved_symbols, references) = collect_symbols_and_references(all_modules)?;

    let public_module_paths: HashSet<String> = all_modules
        .iter()
        .filter(|m| m.is_public)
        .map(|m| m.name.clone())
        .collect();

    resolve_references(
        &mut resolved_symbols,
        references,
        all_modules,
        &public_module_paths,
    );

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

fn collect_symbols_and_references(
    all_modules: &[Module],
) -> Result<(HashMap<String, ResolvedSymbol>, Vec<SymbolReference>), ExtractionError> {
    let mut resolved_symbols: HashMap<String, ResolvedSymbol> = HashMap::new();
    let mut references: Vec<SymbolReference> = Vec::new();

    for module in all_modules {
        for symbol in &module.symbols {
            match symbol {
                ModuleItem::Symbol { symbol } => {
                    let symbol_path = get_symbol_path(&symbol.name, module);
                    resolved_symbols.insert(
                        symbol_path.clone(),
                        ResolvedSymbol {
                            symbol: symbol.clone(),
                            modules: vec![module.name.clone()],
                        },
                    );
                }
                ModuleItem::SymbolReexport {
                    source_path,
                    import_type,
                } => {
                    let normalised_path = normalise_reference(source_path, &module.name)?;
                    references.push(SymbolReference {
                        source_path: normalised_path,
                        referencing_module: module.name.clone(),
                        import_type: import_type.clone(),
                    });
                }
            }
        }
    }
    Ok((resolved_symbols, references))
}

fn resolve_references(
    resolved_symbols: &mut HashMap<String, ResolvedSymbol>,
    references: Vec<SymbolReference>,
    all_modules: &[Module],
    public_module_paths: &HashSet<String>,
) {
    let mut resolved_count = 0;
    let mut pending_references = references;

    while resolved_count < pending_references.len() {
        let mut new_resolved_count = resolved_count;
        let mut new_references = Vec::new();

        for i in resolved_count..pending_references.len() {
            let reference = &pending_references[i];

            match &reference.import_type {
                ImportType::Simple => {
                    if let Some(resolved) = resolved_symbols.get_mut(&reference.source_path) {
                        let mut new_modules = resolved.modules.clone();
                        new_modules.push(reference.referencing_module.clone());
                        let new_modules_set: HashSet<_> = new_modules.into_iter().collect();
                        resolved.modules = new_modules_set.into_iter().collect();
                        new_resolved_count += 1;
                    } else {
                        // Try to find through reference chain or create as external
                        let mut found = false;
                        for (other_path, other_symbol) in resolved_symbols.iter() {
                            if other_path.ends_with(&format!(
                                "::{}",
                                reference.source_path.split("::").last().unwrap()
                            )) {
                                let resolved_symbol = ResolvedSymbol {
                                    symbol: other_symbol.symbol.clone(),
                                    modules: vec![reference.referencing_module.clone()],
                                };
                                resolved_symbols
                                    .insert(reference.source_path.clone(), resolved_symbol);
                                found = true;
                                new_resolved_count += 1;
                                break;
                            }
                        }

                        if !found {
                            let symbol_name = reference.source_path.split("::").last().unwrap();
                            let resolved_symbol = ResolvedSymbol {
                                symbol: Symbol {
                                    name: symbol_name.to_string(),
                                    source_code: format!("pub use {};", reference.source_path),
                                },
                                modules: vec![reference.referencing_module.clone()],
                            };
                            resolved_symbols.insert(reference.source_path.clone(), resolved_symbol);
                            new_resolved_count += 1;
                        }
                    }
                }
                ImportType::Aliased(alias) => {
                    // Add reference to the original symbol
                    if let Some(resolved) = resolved_symbols.get_mut(&reference.source_path) {
                        let mut new_modules = resolved.modules.clone();
                        new_modules.push(reference.referencing_module.clone());
                        let new_modules_set: HashSet<_> = new_modules.into_iter().collect();
                        resolved.modules = new_modules_set.into_iter().collect();
                    }

                    // Create new symbol for the alias
                    let alias_path = if reference.referencing_module.is_empty() {
                        alias.clone()
                    } else {
                        format!("{}::{}", reference.referencing_module, alias)
                    };
                    let symbol = Symbol {
                        name: alias.clone(),
                        source_code: format!("pub use {} as {};", reference.source_path, alias),
                    };
                    resolved_symbols.insert(
                        alias_path,
                        ResolvedSymbol {
                            symbol,
                            modules: vec![reference.referencing_module.clone()],
                        },
                    );
                    new_resolved_count += 1;
                }
                ImportType::Wildcard => {
                    let module_path = get_wildcard_module_path(
                        &reference.source_path,
                        &reference.referencing_module,
                    );
                    if let Some(referenced_module) =
                        all_modules.iter().find(|m| m.name == module_path)
                    {
                        // Add the referencing module to all symbols in resolved_symbols that are from this module or its submodules
                        for (symbol_path, resolved) in resolved_symbols.iter_mut() {
                            if symbol_path.starts_with(&referenced_module.name) {
                                let mut new_modules = resolved.modules.clone();
                                new_modules.push(reference.referencing_module.clone());
                                let new_modules_set: HashSet<_> = new_modules.into_iter().collect();
                                resolved.modules = new_modules_set.into_iter().collect();
                            }
                        }

                        // Process all reexports in the referenced module
                        for symbol in referenced_module.symbols.iter() {
                            if let ModuleItem::SymbolReexport {
                                source_path,
                                import_type,
                            } = symbol
                            {
                                let normalised_path =
                                    normalise_reference(source_path, &referenced_module.name)
                                        .expect(
                                            "Already validated in collect_symbols_and_references",
                                        );
                                new_references.push(SymbolReference {
                                    source_path: normalised_path,
                                    referencing_module: reference.referencing_module.clone(),
                                    import_type: import_type.clone(),
                                });
                            }
                        }
                        new_resolved_count += 1;
                    } else {
                        // Check if we have any symbols that match this path
                        let has_matching_symbols = resolved_symbols
                            .keys()
                            .any(|k| k.contains(&reference.source_path));
                        if !has_matching_symbols {
                            // Create a symbol for the missing module
                            let symbol = Symbol {
                                name: reference
                                    .source_path
                                    .split("::")
                                    .last()
                                    .unwrap_or(&reference.source_path)
                                    .to_string(),
                                source_code: format!("pub use {}::*;", reference.source_path),
                            };
                            resolved_symbols.insert(
                                reference.source_path.clone(),
                                ResolvedSymbol {
                                    symbol,
                                    modules: vec![reference.referencing_module.clone()],
                                },
                            );
                        }
                        new_resolved_count += 1;
                    }
                }
            }
        }

        if new_resolved_count == resolved_count && new_references.is_empty() {
            break;
        }
        resolved_count = new_resolved_count;
        pending_references.extend(new_references);
    }

    // Filter out private modules from each symbol's modules list
    for resolved in resolved_symbols.values_mut() {
        let public_modules: Vec<_> = resolved
            .modules
            .iter()
            .filter(|m| public_module_paths.contains(*m))
            .cloned()
            .collect();
        resolved.modules = public_modules;
    }
}

fn get_symbol_path(symbol_name: &str, module: &Module) -> String {
    if module.name.is_empty() {
        symbol_name.to_string()
    } else {
        format!("{}::{}", module.name, symbol_name)
    }
}

fn normalise_reference(reference: &str, current_module: &str) -> Result<String, ExtractionError> {
    if let Some(stripped) = reference.strip_prefix("crate::") {
        Ok(stripped.to_string())
    } else if let Some(stripped) = reference.strip_prefix("super::") {
        if current_module.is_empty() {
            return Err(ExtractionError::Malformed(
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

fn get_wildcard_module_path(module_path: &str, current_module: &str) -> String {
    if current_module.is_empty() {
        module_path.to_string()
    } else if module_path.contains("::") {
        // If the module path already contains ::, it's a full path
        module_path.to_string()
    } else {
        format!("{}::{}", current_module, module_path)
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
                .expect(&format!("No matching symbol found in {:?}", self.symbols))
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
                is_public: true,
                doc_comment: None,
                symbols: vec![ModuleItem::Symbol {
                    symbol: symbol.clone(),
                }],
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
                is_public: true,
                doc_comment: None,
                symbols: vec![ModuleItem::Symbol {
                    symbol: symbol.clone(),
                }],
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
        use super::*;
        use crate::test_helpers::{stub_symbol, stub_symbol_with_name};

        #[test]
        fn module_via_submodule() {
            let symbol = stub_symbol();
            let modules = vec![
                Module {
                    name: String::new(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "module::test".to_string(),
                        import_type: ImportType::Simple,
                    }],
                },
                Module {
                    name: "module".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: symbol.clone(),
                    }],
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(resolution.get_symbol_modules(symbol), vec![String::new()]);
        }

        #[test]
        fn symbol_via_private_module_block() {
            let symbol = stub_symbol();
            let modules = vec![
                Module {
                    name: String::new(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "priv::test".to_string(),
                        import_type: ImportType::Simple,
                    }],
                },
                Module {
                    name: "priv".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: symbol.clone(),
                    }],
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(resolution.get_symbol_modules(symbol), vec![String::new()]);
        }

        #[test]
        fn partial_private_module_reexport() {
            let reexported_symbol = stub_symbol_with_name("reexported");
            let non_reexported_symbol = stub_symbol_with_name("non_reexported");
            let modules = vec![
                Module {
                    name: String::new(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: format!("inner::{}", reexported_symbol.name),
                        import_type: ImportType::Simple,
                    }],
                },
                Module {
                    name: "inner".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![
                        ModuleItem::Symbol {
                            symbol: reexported_symbol.clone(),
                        },
                        ModuleItem::Symbol {
                            symbol: non_reexported_symbol.clone(),
                        },
                    ],
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
        fn clashing_reexports() {
            let foo_symbol = stub_symbol_with_name("test");
            let bar_symbol = Symbol {
                name: "test".to_string(),
                source_code: "pub fn test() -> i32;".to_string(),
            };
            let modules = vec![
                Module {
                    name: "foo".to_string(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: foo_symbol.clone(),
                    }],
                },
                Module {
                    name: "bar".to_string(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: bar_symbol.clone(),
                    }],
                },
                Module {
                    name: "reexporter1".to_string(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "foo::test".to_string(),
                        import_type: ImportType::Simple,
                    }],
                },
                Module {
                    name: "reexporter2".to_string(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "bar::test".to_string(),
                        import_type: ImportType::Simple,
                    }],
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
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "crate::inner::test".to_string(),
                        import_type: ImportType::Simple,
                    }],
                },
                Module {
                    name: "inner".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: symbol.clone(),
                    }],
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
                is_public: true,
                doc_comment: None,
                symbols: vec![ModuleItem::SymbolReexport {
                    source_path: "super::test".to_string(),
                    import_type: ImportType::Simple,
                }],
            }];

            let result = resolve_symbols(&modules);

            assert!(matches!(
                result,
                Err(ExtractionError::Malformed(msg)) if msg == "Cannot use super from the root module"
            ));
        }

        #[test]
        fn super_path_from_child() {
            let symbol = stub_symbol();
            let modules = vec![
                Module {
                    name: "".to_string(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: symbol.clone(),
                    }],
                },
                Module {
                    name: "child".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "super::test".to_string(),
                        import_type: ImportType::Simple,
                    }],
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
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: symbol.clone(),
                    }],
                },
                Module {
                    name: "parent::child".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "super::test".to_string(),
                        import_type: ImportType::Simple,
                    }],
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
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "self::child::test".to_string(),
                        import_type: ImportType::Simple,
                    }],
                },
                Module {
                    name: "child".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: symbol.clone(),
                    }],
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
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "self::inner::test".to_string(),
                        import_type: ImportType::Simple,
                    }],
                },
                Module {
                    name: "module::inner".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: symbol.clone(),
                    }],
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
        fn simple_nested() {
            let symbol = stub_symbol_with_name("Foo");
            let modules = vec![
                Module {
                    name: String::new(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "child::Foo".to_string(),
                        import_type: ImportType::Simple,
                    }],
                },
                Module {
                    name: "child".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "grandchild::Foo".to_string(),
                        import_type: ImportType::Simple,
                    }],
                },
                Module {
                    name: "child::grandchild".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: symbol.clone(),
                    }],
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            assert_set_eq!(resolution.get_symbol_modules(symbol), vec![String::new()]);
        }

        #[test]
        fn simple_missing() {
            let reference_source_code = "missing::test";
            let modules = vec![Module {
                name: "outer".to_string(),
                is_public: true,
                doc_comment: None,
                symbols: vec![ModuleItem::SymbolReexport {
                    source_path: reference_source_code.to_string(),
                    import_type: ImportType::Simple,
                }],
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
        fn aliased_direct() {
            let original_symbol = stub_symbol_with_name("test");
            let modules = vec![
                Module {
                    name: "reexporter".to_string(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "inner::test".to_string(),
                        import_type: ImportType::Aliased("aliased_test".to_string()),
                    }],
                },
                Module {
                    name: "inner".to_string(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: original_symbol.clone(),
                    }],
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 2);
            let original = resolution
                .symbols
                .iter()
                .find(|s| s.symbol.name == "test")
                .unwrap();
            let aliased = resolution
                .symbols
                .iter()
                .find(|s| s.symbol.name == "aliased_test")
                .unwrap();
            assert_eq!(original.symbol, original_symbol);
            assert_eq!(
                aliased.symbol.source_code,
                "pub use inner::test as aliased_test;"
            );
            assert_set_eq!(
                original.modules,
                vec!["inner".to_string(), "reexporter".to_string()]
            );
            assert_set_eq!(aliased.modules, vec!["reexporter".to_string()]);
        }

        #[test]
        fn aliased_missing() {
            let reference_source_code = "missing::test";
            let alias = "aliased_test";
            let modules = vec![Module {
                name: "outer".to_string(),
                is_public: true,
                doc_comment: None,
                symbols: vec![ModuleItem::SymbolReexport {
                    source_path: reference_source_code.to_string(),
                    import_type: ImportType::Aliased(alias.to_string()),
                }],
            }];

            let result = resolve_symbols(&modules).unwrap();

            assert_eq!(result.symbols.len(), 1);
            let resolved_symbol = result.symbols[0].clone();
            assert_eq!(
                resolved_symbol.symbol.source_code,
                format!("pub use {} as {};", reference_source_code, alias)
            );
            assert_set_eq!(resolved_symbol.modules, vec!["outer".to_string()]);
        }

        #[test]
        fn wildcard_direct() {
            let symbol1 = stub_symbol_with_name("one");
            let symbol2 = stub_symbol_with_name("two");
            let modules = vec![
                Module {
                    name: String::new(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "inner".to_string(),
                        import_type: ImportType::Wildcard,
                    }],
                },
                Module {
                    name: "inner".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![
                        ModuleItem::Symbol {
                            symbol: symbol1.clone(),
                        },
                        ModuleItem::Symbol {
                            symbol: symbol2.clone(),
                        },
                    ],
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 2);
            assert_set_eq!(resolution.get_symbol_modules(symbol1), vec![String::new()]);
            assert_set_eq!(resolution.get_symbol_modules(symbol2), vec![String::new()]);
        }

        #[test]
        fn wildcard_nested() {
            let symbol1 = stub_symbol_with_name("One");
            let symbol2 = stub_symbol_with_name("Two");
            let modules = vec![
                Module {
                    name: String::new(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "child".to_string(),
                        import_type: ImportType::Wildcard,
                    }],
                },
                Module {
                    name: "child".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "grandchild".to_string(),
                        import_type: ImportType::Wildcard,
                    }],
                },
                Module {
                    name: "child::grandchild".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![
                        ModuleItem::Symbol {
                            symbol: symbol1.clone(),
                        },
                        ModuleItem::Symbol {
                            symbol: symbol2.clone(),
                        },
                    ],
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 2);
            assert_set_eq!(resolution.get_symbol_modules(symbol1), vec![String::new()]);
            assert_set_eq!(resolution.get_symbol_modules(symbol2), vec![String::new()]);
        }

        #[test]
        fn wildcard_missing() {
            let reference_source_code = "missing";
            let modules = vec![Module {
                name: "outer".to_string(),
                is_public: true,
                doc_comment: None,
                symbols: vec![ModuleItem::SymbolReexport {
                    source_path: reference_source_code.to_string(),
                    import_type: ImportType::Wildcard,
                }],
            }];

            let result = resolve_symbols(&modules).unwrap();

            assert_eq!(result.symbols.len(), 1);
            let resolved_symbol = result.symbols[0].clone();
            assert_eq!(
                resolved_symbol.symbol.source_code,
                format!("pub use {}::*;", reference_source_code)
            );
            assert_set_eq!(resolved_symbol.modules, vec!["outer".to_string()]);
        }
    }

    mod doc_comments {
        use super::*;

        #[test]
        fn namespace_without_doc_comment() {
            let modules = vec![Module {
                name: "text".to_string(),
                is_public: true,
                doc_comment: None,
                symbols: Vec::new(),
            }];

            let resolution = resolve_symbols(&modules).unwrap();

            assert!(resolution.doc_comments.is_empty());
        }

        #[test]
        fn namespace_with_doc_comment() {
            let modules = vec![Module {
                name: "text".to_string(),
                is_public: true,
                doc_comment: Some("Module for text processing".to_string()),
                symbols: Vec::new(),
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
