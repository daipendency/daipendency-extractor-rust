use std::path::Path;

use cargo_metadata::MetadataCommand;
use daipendency_extractor::DependencyResolutionError;

pub fn resolve_dependency_path(
    dependency_name: &str,
    dependant_path: &Path,
) -> Result<std::path::PathBuf, DependencyResolutionError> {
    let manifest_path = dependant_path.join("Cargo.toml");
    let metadata = MetadataCommand::new()
        .manifest_path(manifest_path)
        .exec()
        .map_err(|e| DependencyResolutionError::RetrievalFailure(e.to_string()))?;

    metadata
        .packages
        .iter()
        .find(|package| package.name == dependency_name)
        .map(|package| package.manifest_path.clone().into())
        .ok_or_else(|| DependencyResolutionError::MissingDependency(dependency_name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use assertables::{assert_contains, assert_ok};
    use tempfile::TempDir;

    #[test]
    fn finds_dependency_manifest() {
        let cargo_toml = Path::new(env!("CARGO_MANIFEST_DIR"));
        let dependency_name = "tree-sitter";

        let result = resolve_dependency_path(dependency_name, &cargo_toml);

        assert_ok!(&result);
        assert_contains!(result.unwrap().to_str().unwrap(), dependency_name);
    }

    #[test]
    fn missing_dependency() {
        let cargo_toml = Path::new(env!("CARGO_MANIFEST_DIR"));

        let result = resolve_dependency_path("non-existent-dependency", &cargo_toml);

        assert!(matches!(
            result,
            Err(DependencyResolutionError::MissingDependency(name)) if name == "non-existent-dependency"
        ));
    }

    #[test]
    fn io_error() {
        let temp_dir = TempDir::new().unwrap();
        let non_existent_path = temp_dir.path().join("non-existent").join("Cargo.toml");

        let result = resolve_dependency_path("tree-sitter", &non_existent_path);

        assert!(matches!(
            result,
            Err(DependencyResolutionError::RetrievalFailure(_))
        ));
    }
}
