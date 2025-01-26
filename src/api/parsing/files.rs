use daipendency_extractor::Symbol;

#[derive(Debug, Clone)]
pub struct RustFile {
    pub doc_comment: Option<String>,
    pub symbols: Vec<RustSymbol>,
}

#[derive(Debug, Clone)]
pub enum RustSymbol {
    Symbol {
        symbol: Symbol,
    },
    Module {
        name: String,
        content: Vec<RustSymbol>,
        doc_comment: Option<String>,
    },
    ModuleImport {
        name: String,
        is_reexported: bool,
    },
    SymbolReexport {
        source_path: String,
        is_wildcard: bool,
        /// The alias of the reexported symbol, if any
        alias: Option<String>,
    },
}

#[cfg(test)]
impl RustFile {
    pub fn get_module<'a>(&'a self, path: &str) -> Option<&'a [RustSymbol]> {
        let parts: Vec<&str> = path.split("::").collect();
        let mut current_symbols = &self.symbols;

        for part in parts {
            let module_symbols = current_symbols.iter().find_map(|symbol| {
                if let RustSymbol::Module { name, content, .. } = symbol {
                    if name == part {
                        Some(content)
                    } else {
                        None
                    }
                } else {
                    None
                }
            });
            match module_symbols {
                Some(next_symbols) => current_symbols = next_symbols,
                None => return None,
            }
        }

        Some(current_symbols)
    }

    pub fn get_symbol<'a>(&'a self, path: &str) -> Option<&'a RustSymbol> {
        let parts: Vec<&str> = path.split("::").collect();
        if parts.is_empty() {
            return None;
        }

        let (symbol_name, module_path) = if parts.len() == 1 {
            (parts[0], None)
        } else {
            (parts[parts.len() - 1], Some(&parts[..parts.len() - 1]))
        };

        let symbols = if let Some(module_parts) = module_path {
            self.get_module(&module_parts.join("::"))?
        } else {
            &self.symbols
        };

        symbols.iter().find(|s| match s {
            RustSymbol::Symbol { symbol } => symbol.name == symbol_name,
            RustSymbol::Module { name, .. } => name == symbol_name,
            RustSymbol::ModuleImport { name, .. } => name == symbol_name,
            RustSymbol::SymbolReexport { source_path, .. } => {
                source_path.split("::").last().unwrap() == symbol_name
            }
        })
    }
}
