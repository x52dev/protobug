use camino::Utf8PathBuf;
use error_stack::Report;

pub fn validate_schema(
    schema_path: Utf8PathBuf,
) -> std::result::Result<String, Report<anyhow::Error>> {
    crate::schema::validate_schema(schema_path)
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use super::*;

    fn schema_path() -> Utf8PathBuf {
        Utf8PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../protogen/proto/system-event.proto"
        ))
    }

    #[test]
    fn validate_schema_returns_the_requested_schema_descriptor() {
        let descriptor = validate_schema(schema_path()).unwrap();

        assert!(descriptor.contains("name: \"SystemEvent\""));
        assert!(!descriptor.contains("name: \"Timestamp\""));
    }
}
