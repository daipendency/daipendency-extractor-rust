#![cfg(test)]

use super::symbol_collection::Module;

pub fn get_module<'a>(name: &str, modules: &'a [Module]) -> Option<&'a Module> {
    modules.iter().find(|m| m.name == name)
}
