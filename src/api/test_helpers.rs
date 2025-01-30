#![cfg(test)]

use super::module_directory::ModuleDirectory;

pub fn get_module_directory<'a>(
    name: &str,
    directories: &'a [ModuleDirectory],
) -> Option<&'a ModuleDirectory> {
    directories.iter().find(|m| m.name == name)
}
