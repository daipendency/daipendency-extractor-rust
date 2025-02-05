use std::collections::HashMap;

use daipendency_extractor::{ExtractionError, Symbol};

use super::parsing::{ImportType, RustFile, RustSymbol};

#[derive(Debug, Clone, PartialEq)]
pub enum ModuleItem {
    /// A public symbol (e.g. `pub struct Foo { ... }`)
    Symbol { symbol: Symbol },
    /// A symbol reexport (e.g. `pub use foo::Bar;`)
    SymbolReexport {
        source_path: String,
        import_type: ImportType,
    },
}

#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub is_public: bool,
    pub doc_comment: Option<String>,
    pub symbols: Vec<ModuleItem>,
}

/// A module directory like `src` (with `src/lib.rs`) or `src/submodule` (with `src/submodule/mod.rs`).
#[derive(Debug, Clone)]
pub struct ModuleDirectory {
    /// The name of the module directory.
    ///
    /// For example, "" for the crate root, "submodule" for `src/submodule/mod.rs` and "submodule::grandchild" for `src/submodule/grandchild.rs`.
    pub name: String,
    /// Whether the module directory is public (i.e. its parent reexports it).
    pub is_public: bool,
    /// The entry point of the module directory.
    ///
    /// For example, `src/lib.rs` or `src/submodule/mod.rs`.
    pub entry_point: RustFile,
    /// The internal files of the module directory.
    ///
    /// For example, `src/submodule.rs` or `src/submodule/another_submodule.rs`.
    pub internal_files: HashMap<String, RustFile>,
}

impl ModuleDirectory {
    pub fn extract_modules(&self) -> Result<Vec<Module>, ExtractionError> {
        extract_modules_from_symbols(
            &self.name,
            self.is_public,
            self.entry_point.doc_comment.clone(),
            &self.entry_point.symbols,
            &self.internal_files,
        )
    }
}

fn extract_modules_from_symbols(
    root_module_name: &str,
    root_module_is_public: bool,
    root_module_doc_comment: Option<String>,
    symbols: &Vec<RustSymbol>,
    internal_files: &HashMap<String, RustFile>,
) -> Result<Vec<Module>, ExtractionError> {
    let mut root_module = Module {
        name: root_module_name.to_string(),
        symbols: Vec::new(),
        doc_comment: root_module_doc_comment,
        is_public: root_module_is_public,
    };
    let mut root_symbols: Vec<ModuleItem> = Vec::new();
    let mut submodules = vec![];
    for symbol in symbols {
        match symbol {
            RustSymbol::ModuleBlock {
                name,
                content,
                doc_comment,
                is_public,
            } => {
                let nested_module_name = get_symbol_path(name, &root_module);
                let nested_modules = extract_modules_from_symbols(
                    &nested_module_name,
                    *is_public,
                    doc_comment.clone(),
                    content,
                    &HashMap::new(),
                )?;
                submodules.extend(nested_modules);
            }
            RustSymbol::ModuleImport {
                name,
                is_reexported,
            } => {
                if let Some(file) = internal_files.get(name) {
                    let internal_file_modules = extract_modules_from_symbols(
                        &get_symbol_path(name, &root_module),
                        *is_reexported,
                        file.doc_comment.clone(),
                        &file.symbols,
                        &HashMap::new(),
                    )?;
                    submodules.extend(internal_file_modules);
                }
            }
            RustSymbol::Symbol { symbol } => {
                root_symbols.push(ModuleItem::Symbol {
                    symbol: symbol.clone(),
                });
            }
            RustSymbol::Reexport {
                source_path,
                import_type,
            } => {
                root_symbols.push(ModuleItem::SymbolReexport {
                    source_path: source_path.clone(),
                    import_type: import_type.clone(),
                });
            }
        }
    }
    root_module.symbols = root_symbols;
    submodules.insert(0, root_module);
    Ok(submodules)
}

fn get_symbol_path(symbol_name: &str, module: &Module) -> String {
    if module.name.is_empty() {
        symbol_name.to_string()
    } else {
        format!("{}::{}", module.name, symbol_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::stub_symbol_with_name;

    const STUB_SYMBOL_NAME: &str = "test";

    fn stub_rust_symbol(symbol: Symbol) -> RustSymbol {
        RustSymbol::Symbol { symbol }
    }

    fn stub_module_item(symbol: Symbol) -> ModuleItem {
        ModuleItem::Symbol { symbol }
    }

    mod module_extraction {
        use assertables::assert_matches;

        use crate::{api::parsing::ImportType, test_helpers::stub_symbol};

        use super::*;

        #[test]
        fn name() {
            let name = "src".to_string();
            let directory = ModuleDirectory {
                name: name.clone(),
                is_public: true,
                entry_point: RustFile {
                    doc_comment: None,
                    symbols: vec![],
                },
                internal_files: HashMap::new(),
            };

            let modules = directory.extract_modules().unwrap();

            assert_eq!(modules.len(), 1);
            assert_eq!(modules[0].name, name);
        }

        #[test]
        fn doc_comment() {
            let doc_comment = Some("This is a doc comment".to_string());
            let directory = ModuleDirectory {
                name: String::new(),
                is_public: true,
                entry_point: RustFile {
                    doc_comment: doc_comment.clone(),
                    symbols: vec![],
                },
                internal_files: HashMap::new(),
            };

            let modules = directory.extract_modules().unwrap();

            assert_eq!(modules.len(), 1);
            assert_eq!(modules[0].doc_comment, doc_comment);
        }

        #[test]
        fn symbol() {
            let symbol = stub_symbol_with_name(STUB_SYMBOL_NAME);
            let directory = ModuleDirectory {
                name: String::new(),
                is_public: true,
                entry_point: RustFile {
                    doc_comment: None,
                    symbols: vec![stub_rust_symbol(symbol.clone())],
                },
                internal_files: HashMap::new(),
            };

            let modules = directory.extract_modules().unwrap();

            assert_eq!(modules.len(), 1);
            let module = &modules[0];
            assert_eq!(module.name, "");
            assert_eq!(module.symbols.len(), 1);
            assert_eq!(module.symbols[0], stub_module_item(symbol));
        }

        #[test]
        fn symbol_reexport() {
            let original_symbol = stub_symbol();
            let directory = ModuleDirectory {
                name: String::new(),
                is_public: true,
                entry_point: RustFile {
                    symbols: vec![
                        RustSymbol::ModuleImport {
                            name: "submodule".to_string(),
                            is_reexported: false,
                        },
                        RustSymbol::Reexport {
                            source_path: "submodule::test".to_string(),
                            import_type: ImportType::Simple,
                        },
                    ],
                    doc_comment: None,
                },
                internal_files: HashMap::from([(
                    "submodule".to_string(),
                    RustFile {
                        symbols: vec![stub_rust_symbol(original_symbol.clone())],
                        doc_comment: None,
                    },
                )]),
            };

            let modules = directory.extract_modules().unwrap();

            assert_eq!(modules.len(), 2);
            let root = &modules[0];
            assert_eq!(root.symbols.len(), 1);
            assert_matches!(
                &root.symbols[0],
                ModuleItem::SymbolReexport {
                    source_path,
                    import_type: ImportType::Simple
                } if source_path == "submodule::test"
            );
            let submodule = &modules[1];
            assert_eq!(submodule.name, "submodule");
            assert_eq!(submodule.symbols.len(), 1);
            assert_matches!(
                &submodule.symbols[0],
                ModuleItem::Symbol { symbol } if symbol.name == original_symbol.name
            );
        }

        mod visibility {
            use super::*;

            #[test]
            fn public_module_directory() {
                let directory = ModuleDirectory {
                    name: String::new(),
                    is_public: true,
                    entry_point: RustFile {
                        doc_comment: None,
                        symbols: vec![],
                    },
                    internal_files: HashMap::new(),
                };

                let modules = directory.extract_modules().unwrap();

                assert_eq!(modules.len(), 1);
                assert!(modules[0].is_public);
            }

            #[test]
            fn private_module_directory() {
                let directory = ModuleDirectory {
                    name: String::new(),
                    is_public: false,
                    entry_point: RustFile {
                        doc_comment: None,
                        symbols: vec![],
                    },
                    internal_files: HashMap::new(),
                };

                let modules = directory.extract_modules().unwrap();

                assert_eq!(modules.len(), 1);
                assert!(!modules[0].is_public);
            }
        }

        mod module_blocks {
            use super::*;

            #[test]
            fn public_module_block() {
                let symbol = stub_symbol_with_name(STUB_SYMBOL_NAME);
                let directory = ModuleDirectory {
                    name: String::new(),
                    is_public: true,
                    entry_point: RustFile {
                        doc_comment: None,
                        symbols: vec![RustSymbol::ModuleBlock {
                            name: "public_mod".to_string(),
                            content: vec![stub_rust_symbol(symbol.clone())],
                            doc_comment: None,
                            is_public: true,
                        }],
                    },
                    internal_files: HashMap::new(),
                };

                let modules = directory.extract_modules().unwrap();

                assert_eq!(modules.len(), 2);
                let submodule = &modules[1];
                assert_eq!(submodule.name, "public_mod");
                assert!(submodule.is_public);
                assert_eq!(submodule.symbols.len(), 1);
                assert_eq!(submodule.symbols[0], stub_module_item(symbol));
            }

            #[test]
            fn public_nested_module_block() {
                let symbol = stub_symbol_with_name(STUB_SYMBOL_NAME);
                let directory = ModuleDirectory {
                    name: String::new(),
                    is_public: true,
                    entry_point: RustFile {
                        doc_comment: None,
                        symbols: vec![RustSymbol::ModuleBlock {
                            name: "parent".to_string(),
                            content: vec![RustSymbol::ModuleBlock {
                                name: "child".to_string(),
                                content: vec![stub_rust_symbol(symbol.clone())],
                                doc_comment: None,
                                is_public: true,
                            }],
                            doc_comment: None,
                            is_public: true,
                        }],
                    },
                    internal_files: HashMap::new(),
                };

                let modules = directory.extract_modules().unwrap();

                assert_eq!(modules.len(), 3);
                assert_matches!(&modules[0],Module { name, .. } if name == "");
                assert_matches!(&modules[1],Module { name, .. } if name == "parent");
                assert_matches!(
                    &modules[2],
                    Module { name, symbols, .. } if name == "parent::child" && symbols.len() == 1 && symbols[0] == stub_module_item(symbol)
                );
            }

            #[test]
            fn private_module_block() {
                let symbol = stub_symbol_with_name(STUB_SYMBOL_NAME);
                let directory = ModuleDirectory {
                    name: String::new(),
                    is_public: true,
                    entry_point: RustFile {
                        doc_comment: None,
                        symbols: vec![RustSymbol::ModuleBlock {
                            name: "private_mod".to_string(),
                            content: vec![stub_rust_symbol(symbol.clone())],
                            doc_comment: None,
                            is_public: false,
                        }],
                    },
                    internal_files: HashMap::new(),
                };

                let modules = directory.extract_modules().unwrap();

                assert_eq!(modules.len(), 2);
                let submodule = &modules[1];
                assert_eq!(submodule.name, "private_mod");
                assert!(!submodule.is_public);
                assert_eq!(submodule.symbols.len(), 1);
                assert_eq!(submodule.symbols[0], stub_module_item(symbol));
            }
        }

        mod module_imports {
            use super::*;

            #[test]
            fn module_reexport() {
                let symbol = stub_symbol_with_name(STUB_SYMBOL_NAME);
                let directory = ModuleDirectory {
                    name: String::new(),
                    is_public: true,
                    entry_point: RustFile {
                        doc_comment: None,
                        symbols: vec![RustSymbol::ModuleImport {
                            name: "submodule".to_string(),
                            is_reexported: true,
                        }],
                    },
                    internal_files: HashMap::from([(
                        "submodule".to_string(),
                        RustFile {
                            doc_comment: None,
                            symbols: vec![stub_rust_symbol(symbol.clone())],
                        },
                    )]),
                };

                let modules = directory.extract_modules().unwrap();

                assert_eq!(modules.len(), 2);
                let module = &modules[1];
                assert_eq!(module.name, "submodule");
                assert_eq!(module.symbols.len(), 1);
                assert_eq!(module.symbols[0], stub_module_item(symbol));
            }

            #[test]
            fn module_imported_but_not_reexported() {
                let symbol = stub_symbol_with_name(STUB_SYMBOL_NAME);
                let directory = ModuleDirectory {
                    name: String::new(),
                    is_public: true,
                    entry_point: RustFile {
                        doc_comment: None,
                        symbols: vec![RustSymbol::ModuleImport {
                            name: "submodule".to_string(),
                            is_reexported: false,
                        }],
                    },
                    internal_files: HashMap::from([(
                        "submodule".to_string(),
                        RustFile {
                            doc_comment: None,
                            symbols: vec![stub_rust_symbol(symbol.clone())],
                        },
                    )]),
                };

                let modules = directory.extract_modules().unwrap();

                assert_eq!(modules.len(), 2);
                let root = &modules[0];
                assert_eq!(root.name, "");
                assert!(root.is_public);
                assert_eq!(root.symbols.len(), 0);
                let submodule = &modules[1];
                assert_eq!(submodule.name, "submodule");
                assert!(!submodule.is_public);
                assert_eq!(submodule.symbols.len(), 1);
                assert_eq!(submodule.symbols[0], stub_module_item(symbol));
            }

            #[test]
            fn missing_internal_file() {
                let directory = ModuleDirectory {
                    name: String::new(),
                    is_public: true,
                    entry_point: RustFile {
                        doc_comment: None,
                        symbols: vec![RustSymbol::ModuleImport {
                            name: "missing_module".to_string(),
                            is_reexported: true,
                        }],
                    },
                    internal_files: HashMap::new(),
                };

                let modules = directory.extract_modules().unwrap();

                assert_eq!(modules.len(), 1);
                let root = &modules[0];
                assert_eq!(root.name, "");
                assert!(root.is_public);
                assert_eq!(root.symbols.len(), 0);
            }
        }
    }
}
