mod api;
mod dependencies;
mod extractor;
mod metadata;
#[cfg(test)]
mod test_helpers;
#[cfg(test)]
mod treesitter_test_helpers;

pub use extractor::RustExtractor;
