use std::fs;

use base64::{Engine as _, prelude::BASE64_STANDARD};
use camino::Utf8PathBuf;
use insta::assert_snapshot;
use pretty_assertions::assert_eq;
use protobuf::{
    EnumOrUnknown, Message as _, MessageField, SpecialFields,
    well_known_types::timestamp::Timestamp,
};
use protogen::system_event::{
    SystemEvent,
    system_event::{Event as SystemEventVariant, MouseButton, MouseDown},
};
use tempfile::tempdir;

use super::*;
use crate::{json, load_inspector};

fn schema_path() -> Utf8PathBuf {
    Utf8PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../protogen/proto/system-event.proto"
    ))
}

fn sample_bytes() -> Vec<u8> {
    SystemEvent {
        timestamp: MessageField::some(Timestamp {
            seconds: 1_234_567,
            nanos: 123,
            special_fields: SpecialFields::default(),
        }),
        reason: Some("user clicked".to_owned()),
        event: Some(SystemEventVariant::Click(MouseDown {
            button: EnumOrUnknown::new(MouseButton::Left),
            x: 42,
            y: 100,
            ..Default::default()
        })),
        special_fields: SpecialFields::default(),
    }
    .write_to_bytes()
    .unwrap()
}

fn sample_json() -> String {
    load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap()
    .canonical_json()
    .unwrap()
}

#[test]
fn json_filter_matches_snapshot() {
    let filtered = json::apply_json_filter(
        &sample_json(),
        r#".reason = "patched with jaq" | .click.button = "Right" | .click.x = 7 | .click.y = 9 | .timestamp.nanos = 456 | .timestamp.seconds = "1234568""#,
    )
    .unwrap();

    assert_snapshot!(filtered);
}

#[test]
fn filtered_edit_bytes_match_snapshot() {
    let dir = tempdir().unwrap();
    let input_path = Utf8PathBuf::from_path_buf(dir.path().join("input.json")).unwrap();
    fs::write(&input_path, sample_json()).unwrap();
    let bytes = edit_to_bytes(EditOptions {
        schema: schema_path(),
        message: Some("SystemEvent".to_owned()),
        file: Some(input_path),
        input_format: InputFormat::Json,
        filter: Some(
            r#".reason = "patched with jaq" | .click.button = "Right" | .click.x = 7 | .click.y = 9 | .timestamp.nanos = 456 | .timestamp.seconds = "1234568""#
                .to_owned(),
        ),
        multiple: false,
    })
    .unwrap();

    assert_snapshot!(hex::encode(&bytes));
}

#[test]
fn edit_binary_input_round_trips_sample_bytes() {
    let dir = tempdir().unwrap();
    let input_path = Utf8PathBuf::from_path_buf(dir.path().join("input.bin")).unwrap();
    fs::write(&input_path, sample_bytes()).unwrap();
    let bytes = edit_to_bytes(EditOptions {
        schema: schema_path(),
        message: Some("SystemEvent".to_owned()),
        file: Some(input_path),
        input_format: InputFormat::Binary,
        filter: None,
        multiple: false,
    })
    .unwrap();

    assert_eq!(bytes, sample_bytes());
}

#[test]
fn edit_hex_input_round_trips_sample_bytes() {
    let dir = tempdir().unwrap();
    let input_path = Utf8PathBuf::from_path_buf(dir.path().join("input.hex")).unwrap();
    fs::write(&input_path, hex::encode(sample_bytes())).unwrap();
    let bytes = edit_to_bytes(EditOptions {
        schema: schema_path(),
        message: Some("SystemEvent".to_owned()),
        file: Some(input_path),
        input_format: InputFormat::Hex,
        filter: None,
        multiple: false,
    })
    .unwrap();

    assert_eq!(bytes, sample_bytes());
}

#[test]
fn edit_base64_input_round_trips_sample_bytes() {
    let dir = tempdir().unwrap();
    let input_path = Utf8PathBuf::from_path_buf(dir.path().join("input.base64")).unwrap();
    fs::write(&input_path, BASE64_STANDARD.encode(sample_bytes())).unwrap();
    let bytes = edit_to_bytes(EditOptions {
        schema: schema_path(),
        message: Some("SystemEvent".to_owned()),
        file: Some(input_path),
        input_format: InputFormat::Base64,
        filter: None,
        multiple: false,
    })
    .unwrap();

    assert_eq!(bytes, sample_bytes());
}

#[test]
fn edit_in_place_preserves_hex_encoding() {
    let dir = tempdir().unwrap();
    let input_path = Utf8PathBuf::from_path_buf(dir.path().join("input.hex")).unwrap();
    fs::write(&input_path, hex::encode(sample_bytes())).unwrap();

    edit_in_place(EditOptions {
        schema: schema_path(),
        message: Some("SystemEvent".to_owned()),
        file: Some(input_path.clone()),
        input_format: InputFormat::Hex,
        filter: Some(r#".click.x = 100 | .click.y = 42"#.to_owned()),
        multiple: false,
    })
    .unwrap();

    let written = fs::read_to_string(&input_path).unwrap();
    assert_snapshot!(written);
}

#[test]
fn edit_in_place_preserves_json_encoding() {
    let dir = tempdir().unwrap();
    let input_path = Utf8PathBuf::from_path_buf(dir.path().join("input.json")).unwrap();
    fs::write(&input_path, sample_json()).unwrap();

    edit_in_place(EditOptions {
        schema: schema_path(),
        message: Some("SystemEvent".to_owned()),
        file: Some(input_path.clone()),
        input_format: InputFormat::Json,
        filter: Some(r#".click.x = 100 | .click.y = 42"#.to_owned()),
        multiple: false,
    })
    .unwrap();

    let written = fs::read_to_string(&input_path).unwrap();
    assert_snapshot!(written);
}

#[test]
fn edit_multiple_hex_lines_match_snapshot() {
    let dir = tempdir().unwrap();
    let input_path = Utf8PathBuf::from_path_buf(dir.path().join("input.hex")).unwrap();
    let bytes = sample_bytes();
    let second = bytes.clone();
    fs::write(
        &input_path,
        format!("{}\n{}\n", hex::encode(&bytes), hex::encode(&second)),
    )
    .unwrap();

    let output = edit_to_encoded_lines(
        EditOptions {
            schema: schema_path(),
            message: Some("SystemEvent".to_owned()),
            file: Some(input_path),
            input_format: InputFormat::Hex,
            filter: Some(r#".click.x = 100 | .click.y = 42"#.to_owned()),
            multiple: true,
        },
        InputFormat::Hex,
    )
    .unwrap();

    assert_snapshot!(output);
}

#[test]
fn edit_multiple_base64_to_json_lines_match_snapshot() {
    let dir = tempdir().unwrap();
    let input_path = Utf8PathBuf::from_path_buf(dir.path().join("input.base64")).unwrap();
    let bytes = sample_bytes();
    let second = bytes.clone();
    fs::write(
        &input_path,
        format!(
            "{}\n{}\n",
            BASE64_STANDARD.encode(&bytes),
            BASE64_STANDARD.encode(&second)
        ),
    )
    .unwrap();

    let output = edit_to_json_lines(EditOptions {
        schema: schema_path(),
        message: Some("SystemEvent".to_owned()),
        file: Some(input_path),
        input_format: InputFormat::Base64,
        filter: Some(r#".click.x = 100 | .click.y = 42"#.to_owned()),
        multiple: true,
    })
    .unwrap();

    assert_snapshot!(output);
}

#[test]
fn edit_in_place_preserves_multiple_base64_encoding() {
    let dir = tempdir().unwrap();
    let input_path = Utf8PathBuf::from_path_buf(dir.path().join("input.base64")).unwrap();
    let bytes = sample_bytes();
    let second = bytes.clone();
    fs::write(
        &input_path,
        format!(
            "{}\n{}\n",
            BASE64_STANDARD.encode(&bytes),
            BASE64_STANDARD.encode(&second)
        ),
    )
    .unwrap();

    edit_in_place(EditOptions {
        schema: schema_path(),
        message: Some("SystemEvent".to_owned()),
        file: Some(input_path.clone()),
        input_format: InputFormat::Base64,
        filter: Some(r#".click.x = 100 | .click.y = 42"#.to_owned()),
        multiple: true,
    })
    .unwrap();

    let written = fs::read_to_string(&input_path).unwrap();
    assert_snapshot!(written);
}
