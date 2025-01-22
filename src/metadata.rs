use daipendency_extractor::{LibraryMetadata, LibraryMetadataError};
use serde::{de::Error, Deserialize, Serialize};
use std::fs;
use std::path::Path;

const DEFAULT_LIB_PATH: &str = "src/lib.rs";
const README_PATH: &str = "README.md";

#[derive(Debug, Deserialize, Serialize)]
struct PackageConfig {
    name: String,
    #[serde(default, deserialize_with = "deserialize_version")]
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum VersionField {
    Direct(Option<String>),
    #[serde(rename = "workspace")]
    Workspace(serde::de::IgnoredAny),
}

fn deserialize_version<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match VersionField::deserialize(deserializer) {
        Ok(VersionField::Direct(version)) => Ok(version),
        Ok(VersionField::Workspace(_)) => Ok(None),
        Err(e) => Err(D::Error::custom(format!("Malformed version field: {}", e))),
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct LibConfig {
    path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CargoConfig {
    package: PackageConfig,
    lib: Option<LibConfig>,
}

pub fn extract_metadata(path: &Path) -> Result<LibraryMetadata, LibraryMetadataError> {
    let cargo_toml_path = path.join("Cargo.toml");
    let cargo_toml_content =
        fs::read_to_string(&cargo_toml_path).map_err(LibraryMetadataError::MissingManifest)?;

    let cargo_config: CargoConfig = toml::from_str(&cargo_toml_content)
        .map_err(|e| LibraryMetadataError::MalformedManifest(format!("{}", e)))?;

    let readme_path = path.join(README_PATH);
    let documentation = fs::read_to_string(&readme_path).unwrap_or_default();

    let entry_point = cargo_config
        .lib
        .and_then(|lib| lib.path)
        .map(|path_str| path.join(Path::new(&path_str)))
        .unwrap_or_else(|| path.join(DEFAULT_LIB_PATH));

    Ok(LibraryMetadata {
        name: cargo_config.package.name,
        version: cargo_config.package.version,
        documentation,
        entry_point,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_crate(dir: &Path, custom_lib: Option<String>) -> Result<(), std::io::Error> {
        let config = CargoConfig {
            package: PackageConfig {
                name: "test-crate".to_string(),
                version: Some("0.1.0".to_string()),
            },
            lib: custom_lib.map(|path| LibConfig { path: Some(path) }),
        };

        let cargo_toml = toml::to_string(&config).unwrap();
        fs::write(dir.join("Cargo.toml"), cargo_toml)?;
        fs::write(dir.join(README_PATH), "Test crate")?;
        Ok(())
    }

    #[test]
    fn extract_metadata_valid_crate() {
        let temp_dir = TempDir::new().unwrap();
        create_test_crate(temp_dir.path(), None).unwrap();

        let result = extract_metadata(temp_dir.path());

        let metadata = result.unwrap();
        assert_eq!(metadata.name, "test-crate");
        assert_eq!(metadata.version, Some("0.1.0".to_string()));
        assert_eq!(metadata.documentation, "Test crate");
    }

    #[test]
    fn missing_cargo_toml() {
        let temp_dir = TempDir::new().unwrap();

        let result = extract_metadata(temp_dir.path());

        assert!(matches!(
            result,
            Err(LibraryMetadataError::MissingManifest(_))
        ));
    }

    #[test]
    fn missing_version() {
        let temp_dir = TempDir::new().unwrap();
        let config = CargoConfig {
            package: PackageConfig {
                name: "test-crate".to_string(),
                version: None,
            },
            lib: None,
        };
        fs::write(
            temp_dir.path().join("Cargo.toml"),
            toml::to_string(&config).unwrap(),
        )
        .unwrap();
        fs::write(temp_dir.path().join(README_PATH), "Test crate").unwrap();

        let result = extract_metadata(temp_dir.path()).unwrap();

        assert_eq!(result.version, None);
    }

    #[test]
    fn invalid_cargo_toml() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("Cargo.toml"), "invalid toml content").unwrap();

        let result = extract_metadata(temp_dir.path());

        assert!(matches!(
            result,
            Err(LibraryMetadataError::MalformedManifest(_))
        ));
    }

    #[test]
    fn missing_package_section() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(
            temp_dir.path().join("Cargo.toml"),
            "[dependencies]\nfoo = \"1.0\"",
        )
        .unwrap();

        let result = extract_metadata(temp_dir.path());

        assert!(matches!(
            result,
            Err(LibraryMetadataError::MalformedManifest(_))
        ));
    }

    #[test]
    fn missing_readme() {
        let temp_dir = TempDir::new().unwrap();
        create_test_crate(temp_dir.path(), None).unwrap();
        fs::remove_file(temp_dir.path().join(README_PATH)).unwrap();

        let result = extract_metadata(temp_dir.path());

        assert!(result.is_ok());
        assert_eq!(result.unwrap().documentation, "");
    }

    #[test]
    fn workspace_version() {
        let temp_dir = TempDir::new().unwrap();
        let cargo_toml = r#"
[package]
name = "test-crate"
version.workspace = true
"#;
        fs::write(temp_dir.path().join("Cargo.toml"), cargo_toml).unwrap();
        fs::write(temp_dir.path().join(README_PATH), "Test crate").unwrap();

        let metadata = extract_metadata(temp_dir.path()).unwrap();

        assert_eq!(metadata.version, None);
    }

    mod entrypoint {
        use super::*;

        #[test]
        fn default_entry_point() {
            let temp_dir = TempDir::new().unwrap();

            create_test_crate(temp_dir.path(), None).unwrap();

            let metadata = extract_metadata(temp_dir.path()).unwrap();

            assert_eq!(metadata.entry_point, temp_dir.path().join(DEFAULT_LIB_PATH));
        }

        #[test]
        fn custom_entry_point() {
            let temp_dir = TempDir::new().unwrap();
            let custom_lib_path = "src/custom_lib.rs";
            create_test_crate(temp_dir.path(), Some(custom_lib_path.to_string())).unwrap();

            let metadata = extract_metadata(temp_dir.path()).unwrap();

            assert_eq!(metadata.entry_point, temp_dir.path().join(custom_lib_path));
        }
    }
}
