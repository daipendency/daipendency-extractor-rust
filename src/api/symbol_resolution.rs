use daipendency_extractor::ExtractionError;
use daipendency_extractor::Symbol;
use regex::escape;
use regex::Regex;
use std::collections::{HashMap, HashSet};

use super::module_directory::{Module, ModuleItem};
use super::parsing::ImportType;

#[derive(Debug, Clone)]
pub struct SymbolDeclaration {
    pub symbol: Symbol,
    pub modules: Vec<String>,
}

#[derive(Debug)]
pub struct SymbolResolution {
    pub symbols: Vec<SymbolDeclaration>,
    pub doc_comments: HashMap<String, String>,
}

#[derive(Debug)]
struct SymbolReference {
    source_path: String,
    referencing_module: String,
    import_type: ImportType,
}

/// Resolve symbol references by matching them with their corresponding definitions.
pub fn resolve_symbols(modules: &[Module]) -> Result<SymbolResolution, ExtractionError> {
    let symbols = resolve_public_symbols(modules)?;

    let doc_comments = get_doc_comments_by_module(modules);

    Ok(SymbolResolution {
        symbols,
        doc_comments,
    })
}

fn resolve_public_symbols(
    all_modules: &[Module],
) -> Result<Vec<SymbolDeclaration>, ExtractionError> {
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
    )?;

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

    let public_symbols: Vec<SymbolDeclaration> = resolved_symbols
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
) -> Result<(HashMap<String, SymbolDeclaration>, Vec<SymbolReference>), ExtractionError> {
    let mut resolved_symbols: HashMap<String, SymbolDeclaration> = HashMap::new();
    let mut references: Vec<SymbolReference> = Vec::new();

    for module in all_modules {
        for symbol in &module.symbols {
            match symbol {
                ModuleItem::Symbol { symbol } => {
                    let symbol_path = get_symbol_path_from_module(&symbol.name, module);
                    resolved_symbols.insert(
                        symbol_path.clone(),
                        SymbolDeclaration {
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
    all_declarations: &mut HashMap<String, SymbolDeclaration>,
    all_references: Vec<SymbolReference>,
    all_modules: &[Module],
    public_module_paths: &HashSet<String>,
) -> Result<(), ExtractionError> {
    for reference in &all_references {
        let mut visited = HashSet::new();
        let mut declarations = resolve_symbol_reference(
            reference,
            all_declarations,
            &all_references,
            &mut visited,
            all_modules,
        )?;

        if declarations.is_empty() {
            declarations = vec![recreate_reexport(reference)];
        }

        for declaration in declarations {
            match &reference.import_type {
                ImportType::Aliased(alias) => {
                    let alias_key =
                        get_symbol_path_from_module_path(alias, &reference.referencing_module);

                    let mut chain_modules = declaration.modules.clone();
                    chain_modules.push(reference.referencing_module.clone());
                    let all_public_in_chain = chain_modules
                        .iter()
                        .all(|m| public_module_paths.contains(m));

                    let aliased_symbol = SymbolDeclaration {
                        symbol: Symbol {
                            name: alias.clone(),
                            source_code: if all_public_in_chain {
                                format!("pub use {} as {};", reference.source_path, alias)
                            } else {
                                rename_symbol_in_source_code(&declaration, alias)
                            },
                        },
                        modules: vec![reference.referencing_module.clone()],
                    };

                    all_declarations.insert(alias_key, aliased_symbol);
                }
                ImportType::Wildcard => {
                    let key = get_symbol_path_from_module_path(
                        &declaration.symbol.name,
                        &reference.referencing_module,
                    );
                    all_declarations.insert(key, declaration);
                }
                ImportType::Simple => {
                    let key = if reference.referencing_module.is_empty() {
                        reference.source_path.clone()
                    } else {
                        format!(
                            "{}::{}",
                            reference.referencing_module,
                            reference.source_path.split("::").last().unwrap()
                        )
                    };

                    if let Some(existing) = all_declarations.get_mut(&key) {
                        let mut new_modules = existing.modules.clone();
                        new_modules.extend(declaration.modules.iter().cloned());
                        let new_modules_set: HashSet<_> = new_modules.into_iter().collect();
                        existing.modules = new_modules_set.into_iter().collect();
                    } else {
                        let original_key = reference.source_path.clone();
                        if all_declarations.contains_key(&original_key) {
                            all_declarations.remove(&original_key);
                        }
                        all_declarations.insert(key, declaration);
                    }
                }
            }
        }
    }

    Ok(())
}

fn rename_symbol_in_source_code(declaration: &SymbolDeclaration, alias: &String) -> String {
    let old_name = &declaration.symbol.name;
    let old_name_regex = Regex::new(&format!(r"\b{}\b", escape(old_name))).unwrap();
    let new_source_code = old_name_regex
        .replace_all(&declaration.symbol.source_code, alias)
        .to_string();
    new_source_code
}

fn resolve_symbol_reference(
    target_ref: &SymbolReference,
    all_declarations: &HashMap<String, SymbolDeclaration>,
    all_references: &[SymbolReference],
    visited: &mut HashSet<String>,
    all_modules: &[Module],
) -> Result<Vec<SymbolDeclaration>, ExtractionError> {
    if !visited.insert(target_ref.source_path.clone()) {
        return Ok(Vec::new());
    }

    if let ImportType::Wildcard = target_ref.import_type {
        let target_module_path = get_symbol_path_from_module_path(
            &target_ref.source_path,
            &target_ref.referencing_module,
        );
        if let Some(target_module) = all_modules.iter().find(|m| m.name == target_module_path) {
            let mut target_module_declarations = get_module_declarations(
                target_module,
                all_declarations,
                all_references,
                visited,
                all_modules,
            )?;

            for declaration in &mut target_module_declarations {
                declaration
                    .modules
                    .push(target_ref.referencing_module.clone());
            }
            return Ok(target_module_declarations);
        }
    }

    let full_path =
        get_symbol_path_from_module_path(&target_ref.source_path, &target_ref.referencing_module);

    if let Some(declaration) = all_declarations
        .get(&full_path)
        .or_else(|| all_declarations.get(&target_ref.source_path))
    {
        let mut declaration_clone = declaration.clone();
        if !matches!(target_ref.import_type, ImportType::Aliased(_)) {
            declaration_clone
                .modules
                .push(target_ref.referencing_module.clone());
        }
        return Ok(vec![declaration_clone]);
    }

    let mut found_symbols = Vec::new();
    for reference in all_references {
        let target_first_part = target_ref.source_path.split("::").next().unwrap_or("");
        let reference_matches = match &target_ref.import_type {
            ImportType::Simple | ImportType::Aliased(_) => {
                reference.referencing_module == target_first_part
            }
            ImportType::Wildcard => reference.source_path.starts_with(&target_ref.source_path),
        };

        if reference_matches {
            let mut resolved_declarations = resolve_symbol_reference(
                reference,
                all_declarations,
                all_references,
                &mut visited.clone(),
                all_modules,
            )?;
            for declaration in &mut resolved_declarations {
                declaration
                    .modules
                    .push(target_ref.referencing_module.clone());

                if let ImportType::Aliased(alias) = &target_ref.import_type {
                    let original_source = declaration.symbol.source_code.clone();
                    declaration.symbol = Symbol {
                        name: alias.clone(),
                        source_code: original_source,
                    };
                }
            }
            found_symbols.extend(resolved_declarations);
        }
    }

    Ok(found_symbols)
}

fn get_module_declarations(
    target_module: &Module,
    all_declarations: &HashMap<String, SymbolDeclaration>,
    all_references: &[SymbolReference],
    visited: &mut HashSet<String>,
    all_modules: &[Module],
) -> Result<Vec<SymbolDeclaration>, ExtractionError> {
    let mut target_module_declarations = Vec::new();
    for symbol in &target_module.symbols {
        match symbol {
            ModuleItem::Symbol { symbol } => {
                target_module_declarations.push(SymbolDeclaration {
                    symbol: symbol.clone(),
                    modules: vec![target_module.name.clone()],
                });
            }
            ModuleItem::SymbolReexport {
                source_path,
                import_type,
            } => {
                let normalised_path = normalise_reference(source_path, &target_module.name)?;
                let reexport_ref = SymbolReference {
                    source_path: normalised_path,
                    referencing_module: target_module.name.clone(),
                    import_type: import_type.clone(),
                };
                let resolved_declarations = resolve_symbol_reference(
                    &reexport_ref,
                    all_declarations,
                    all_references,
                    &mut visited.clone(),
                    all_modules,
                )?;
                target_module_declarations.extend(resolved_declarations);
            }
        }
    }
    Ok(target_module_declarations)
}

fn recreate_reexport(target_ref: &SymbolReference) -> SymbolDeclaration {
    let modules = vec![target_ref.referencing_module.clone()];
    match &target_ref.import_type {
        ImportType::Simple => {
            let symbol_name = target_ref.source_path.split("::").last().unwrap();
            SymbolDeclaration {
                symbol: Symbol {
                    name: symbol_name.to_string(),
                    source_code: format!("pub use {};", target_ref.source_path),
                },
                modules,
            }
        }
        ImportType::Aliased(alias) => SymbolDeclaration {
            symbol: Symbol {
                name: alias.clone(),
                source_code: format!("pub use {} as {};", target_ref.source_path, alias),
            },
            modules,
        },
        ImportType::Wildcard => SymbolDeclaration {
            symbol: Symbol {
                name: target_ref
                    .source_path
                    .split("::")
                    .last()
                    .unwrap_or(&target_ref.source_path)
                    .to_string(),
                source_code: format!("pub use {}::*;", target_ref.source_path),
            },
            modules,
        },
    }
}

fn get_symbol_path_from_module_path(symbol_name: &str, module_name: &str) -> String {
    if module_name.is_empty() {
        symbol_name.to_string()
    } else {
        format!("{}::{}", module_name, symbol_name)
    }
}

fn get_symbol_path_from_module(symbol_name: &str, module: &Module) -> String {
    get_symbol_path_from_module_path(symbol_name, &module.name)
}

fn normalise_reference(reference: &str, current_module: &str) -> Result<String, ExtractionError> {
    if let Some(stripped) = reference.strip_prefix("crate::") {
        Ok(stripped.to_string())
    } else if let Some(stripped) = reference.strip_prefix("super::") {
        if current_module.is_empty() {
            return Err(ExtractionError::Malformed(format!(
                "Cannot use super from the root module ({})",
                reference
            )));
        }
        if let Some(parent) = current_module.rfind("::") {
            Ok(format!("{}::{}", &current_module[..parent], stripped))
        } else {
            Ok(stripped.to_string())
        }
    } else if let Some(stripped) = reference.strip_prefix("self::") {
        Ok(get_symbol_path_from_module_path(stripped, current_module))
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
                Err(ExtractionError::Malformed(msg)) if msg == "Cannot use super from the root module (super::test)"
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
            assert_set_eq!(original.modules, vec!["inner".to_string()]);
            assert_set_eq!(aliased.modules, vec!["reexporter".to_string()]);
        }

        #[test]
        fn aliased_nested() {
            let symbol = stub_symbol_with_name("Baz");
            let modules = vec![
                Module {
                    name: String::new(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "child::Bar".to_string(),
                        import_type: ImportType::Aliased("Foo".to_string()),
                    }],
                },
                Module {
                    name: "child".to_string(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "grandchild::Baz".to_string(),
                        import_type: ImportType::Aliased("Bar".to_string()),
                    }],
                },
                Module {
                    name: "child::grandchild".to_string(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: symbol.clone(),
                    }],
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 3);

            // Check original declaration
            let original = resolution
                .symbols
                .iter()
                .find(|s| s.symbol.name == "Baz")
                .unwrap();
            assert_eq!(original.symbol, symbol);
            assert_set_eq!(original.modules, vec!["child::grandchild".to_string()]);

            // Check first reexport
            let first_reexport = resolution
                .symbols
                .iter()
                .find(|s| s.symbol.name == "Bar")
                .unwrap();
            assert_eq!(
                first_reexport.symbol.source_code,
                "pub use grandchild::Baz as Bar;"
            );
            assert_set_eq!(first_reexport.modules, vec!["child".to_string()]);

            // Check second reexport
            let second_reexport = resolution
                .symbols
                .iter()
                .find(|s| s.symbol.name == "Foo")
                .unwrap();
            assert_eq!(
                second_reexport.symbol.source_code,
                "pub use child::Bar as Foo;"
            );
            assert_set_eq!(second_reexport.modules, vec![String::new()]);
        }

        #[test]
        fn aliased_via_private_module() {
            let original_symbol = stub_symbol_with_name("Bar");
            let modules = vec![
                Module {
                    name: String::new(),
                    is_public: true,
                    doc_comment: None,
                    symbols: vec![ModuleItem::SymbolReexport {
                        source_path: "child::Bar".to_string(),
                        import_type: ImportType::Aliased("Foo".to_string()),
                    }],
                },
                Module {
                    name: "child".to_string(),
                    is_public: false,
                    doc_comment: None,
                    symbols: vec![ModuleItem::Symbol {
                        symbol: original_symbol.clone(),
                    }],
                },
            ];

            let resolution = resolve_symbols(&modules).unwrap();

            assert_eq!(resolution.symbols.len(), 1);
            let expected_symbol = stub_symbol_with_name("Foo");
            assert_set_eq!(
                resolution.get_symbol_modules(expected_symbol),
                vec![String::new()]
            );
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
