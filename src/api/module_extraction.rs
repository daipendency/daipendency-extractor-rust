use daipendency_extractor::ExtractionError;

use super::module_directory::{Module, ModuleDirectory};

pub fn extract_modules(
    module_directories: &[ModuleDirectory],
) -> Result<Vec<Module>, ExtractionError> {
    let modules = module_directories
        .iter()
        .map(|m| m.extract_modules())
        .collect::<Result<Vec<Vec<Module>>, ExtractionError>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    Ok(modules)
}
