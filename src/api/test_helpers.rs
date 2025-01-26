#![cfg(test)]

use super::symbol_collection::ExternalModule;

pub fn get_module<'a>(name: &str, modules: &'a [ExternalModule]) -> Option<&'a ExternalModule> {
    modules.iter().find(|m| m.name == name)
}
