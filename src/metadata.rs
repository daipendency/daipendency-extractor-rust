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
    use daipendency_testing::tempdir::TempDir;

    fn create_test_crate(
        temp_dir: &TempDir,
        custom_lib: Option<String>,
    ) -> Result<(), std::io::Error> {
        let config = CargoConfig {
            package: PackageConfig {
                name: "test-crate".to_string(),
                version: Some("0.1.0".to_string()),
            },
            lib: custom_lib.map(|path| LibConfig { path: Some(path) }),
        };

        let cargo_toml = toml::to_string(&config).unwrap();
        temp_dir.create_file("Cargo.toml", &cargo_toml)?;
        temp_dir.create_file(README_PATH, "Test crate")?;
        Ok(())
    }

    #[test]
    fn extract_metadata_valid_crate() {
        let temp_dir = TempDir::new();
        create_test_crate(&temp_dir, None).unwrap();
        let dummy = temp_dir.create_file("dummy", "").unwrap();

        let result = extract_metadata(dummy.parent().unwrap());

        let metadata = result.unwrap();
        assert_eq!(metadata.name, "test-crate");
        assert_eq!(metadata.version, Some("0.1.0".to_string()));
        assert_eq!(metadata.documentation, "Test crate");
    }

    #[test]
    fn missing_cargo_toml() {
        let temp_dir = TempDir::new();
        let dummy = temp_dir.create_file("dummy", "").unwrap();

        let result = extract_metadata(dummy.parent().unwrap());

        assert!(matches!(
            result,
            Err(LibraryMetadataError::MissingManifest(_))
        ));
    }

    #[test]
    fn missing_version() {
        let temp_dir = TempDir::new();
        let config = CargoConfig {
            package: PackageConfig {
                name: "test-crate".to_string(),
                version: None,
            },
            lib: None,
        };
        temp_dir
            .create_file("Cargo.toml", &toml::to_string(&config).unwrap())
            .unwrap();
        temp_dir.create_file(README_PATH, "Test crate").unwrap();
        let dummy = temp_dir.create_file("dummy", "").unwrap();

        let result = extract_metadata(dummy.parent().unwrap()).unwrap();

        assert_eq!(result.version, None);
    }

    #[test]
    fn invalid_cargo_toml() {
        let temp_dir = TempDir::new();
        temp_dir
            .create_file("Cargo.toml", "invalid toml content")
            .unwrap();
        let dummy = temp_dir.create_file("dummy", "").unwrap();

        let result = extract_metadata(dummy.parent().unwrap());

        assert!(matches!(
            result,
            Err(LibraryMetadataError::MalformedManifest(_))
        ));
    }

    #[test]
    fn missing_package_section() {
        let temp_dir = TempDir::new();
        temp_dir
            .create_file("Cargo.toml", "[dependencies]\nfoo = \"1.0\"")
            .unwrap();
        let dummy = temp_dir.create_file("dummy", "").unwrap();

        let result = extract_metadata(dummy.parent().unwrap());

        assert!(matches!(
            result,
            Err(LibraryMetadataError::MalformedManifest(_))
        ));
    }

    #[test]
    fn missing_readme() {
        let temp_dir = TempDir::new();
        create_test_crate(&temp_dir, None).unwrap();
        let dummy = temp_dir.create_file("dummy", "").unwrap();

        let result = extract_metadata(dummy.parent().unwrap());

        assert!(result.is_ok());
        assert_eq!(result.unwrap().documentation, "Test crate");
    }

    #[test]
    fn workspace_version() {
        let temp_dir = TempDir::new();
        let cargo_toml = r#"
[package]
name = "test-crate"
version.workspace = true
"#;
        temp_dir.create_file("Cargo.toml", cargo_toml).unwrap();
        temp_dir.create_file(README_PATH, "Test crate").unwrap();
        let dummy = temp_dir.create_file("dummy", "").unwrap();

        let metadata = extract_metadata(dummy.parent().unwrap()).unwrap();

        assert_eq!(metadata.version, None);
    }

    mod entrypoint {
        use super::*;

        #[test]
        fn default_entry_point() {
            let temp_dir = TempDir::new();
            create_test_crate(&temp_dir, None).unwrap();
            let dummy = temp_dir.create_file("dummy", "").unwrap();
            let root_dir = dummy.parent().unwrap();

            let metadata = extract_metadata(root_dir).unwrap();

            assert_eq!(metadata.entry_point, root_dir.join(DEFAULT_LIB_PATH));
        }

        #[test]
        fn custom_entry_point() {
            let temp_dir = TempDir::new();
            let custom_lib_path = "src/custom_lib.rs";
            create_test_crate(&temp_dir, Some(custom_lib_path.to_string())).unwrap();
            let dummy = temp_dir.create_file("dummy", "").unwrap();
            let root_dir = dummy.parent().unwrap();

            let metadata = extract_metadata(root_dir).unwrap();

            assert_eq!(metadata.entry_point, root_dir.join(custom_lib_path));
        }
    }
}
