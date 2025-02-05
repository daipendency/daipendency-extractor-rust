use daipendency_extractor::Symbol;

#[derive(Debug, Clone)]
pub struct RustFile {
    pub doc_comment: Option<String>,
    pub symbols: Vec<RustSymbol>,
}

/// Type of symbol import in a Rust module
#[derive(Debug, Clone, PartialEq)]
pub enum ImportType {
    /// Direct import (e.g. `use submodule::Foo`)
    Simple,
    /// Wildcard import (e.g. `use submodule::*`)
    Wildcard,
    /// Aliased import (e.g. `use submodule::Foo as Bar`)
    Aliased(String),
}

/// The various symbols we care about for the purposes of extracting the public API
#[derive(Debug, Clone, PartialEq)]
pub enum RustSymbol {
    /// A public symbol (e.g. `pub struct Foo { ... }`)
    Symbol { symbol: Symbol },
    /// A module or symbol reexport (e.g. `pub use serde_json;`, `pub use serde_json::Value;`)
    Reexport {
        source_path: String,
        import_type: ImportType,
    },
    /// A module block (e.g. `mod foo { ... }`)
    ModuleBlock {
        name: String,
        is_public: bool,
        content: Vec<RustSymbol>,
        doc_comment: Option<String>,
    },
    /// A module import (e.g. `mod foo;`)
    ModuleImport { name: String, is_reexported: bool },
}

#[cfg(test)]
impl RustFile {
    pub fn get_module<'a>(&'a self, path: &str) -> Option<&'a [RustSymbol]> {
        let parts: Vec<&str> = path.split("::").collect();
        let mut current_symbols = &self.symbols;

        for part in parts {
            let module_symbols = current_symbols.iter().find_map(|symbol| {
                if let RustSymbol::ModuleBlock { name, content, .. } = symbol {
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
            RustSymbol::ModuleBlock { name, .. } => name == symbol_name,
            RustSymbol::ModuleImport { name, .. } => name == symbol_name,
            RustSymbol::Reexport { source_path, .. } => {
                source_path.split("::").last().unwrap() == symbol_name
            }
        })
    }
}
