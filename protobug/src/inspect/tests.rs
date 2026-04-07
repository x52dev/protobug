use std::fs;

use base64::{Engine as _, prelude::BASE64_STANDARD};
use camino::Utf8PathBuf;
use indoc::indoc;
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
use crate::{
    decode,
    message::EnumSelection,
    schema::{available_message_names, load_file_descriptor, select_message},
    selection,
};

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
fn auto_detects_base64_and_hex_input() {
    let bytes = sample_bytes();
    let base64 = BASE64_STANDARD.encode(&bytes);
    let hex = hex::encode(&bytes);

    assert_eq!(
        decode::decode_input(base64.as_bytes(), InputFormat::Auto).unwrap(),
        bytes,
    );
    assert_eq!(
        decode::decode_input(hex.as_bytes(), InputFormat::Auto).unwrap(),
        bytes
    );
}

#[test]
fn read_input_requires_explicit_stdin_marker() {
    let error = decode::read_input(None).unwrap_err();
    let message = format!("{error:?}");

    assert!(message.contains("No input file was provided."));
    assert!(message.contains("Pass --file <path> or --file - to read from stdin."));
}

#[test]
fn explicit_hex_format_ignores_whitespace() {
    let bytes = sample_bytes();
    let spaced_hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");

    assert_eq!(
        decode::decode_input(spaced_hex.as_bytes(), InputFormat::Hex).unwrap(),
        bytes,
    );
}

#[test]
fn selecting_message_is_required_when_schema_has_multiple_messages() {
    let dir = tempdir().unwrap();
    let schema_path = Utf8PathBuf::from_path_buf(dir.path().join("multi.proto")).unwrap();
    fs::write(
        &schema_path,
        indoc! {r#"
            syntax = "proto3";

            message Alpha {}
            message Beta {}
        "#},
    )
    .unwrap();

    let fd = load_file_descriptor(schema_path.as_ref()).unwrap();
    assert_eq!(available_message_names(&fd), vec!["Alpha", "Beta"]);
    assert!(select_message(&fd, None).is_err());
    assert_eq!(select_message(&fd, Some("Beta")).unwrap().name(), "Beta");
}

#[test]
fn inspector_tracks_parse_errors_without_losing_last_valid_message() {
    let mut inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();

    let original = inspector.bytes().unwrap();

    assert!(inspector.apply_json("{ not valid json").is_err());
    assert!(inspector.parse_error().is_some());
    assert_eq!(inspector.bytes().unwrap(), original);

    let mut value =
        serde_json::from_str::<serde_json::Value>(&inspector.canonical_json().unwrap()).unwrap();
    value["reason"] = serde_json::Value::String("updated".to_owned());

    inspector
        .apply_json(&serde_json::to_string_pretty(&value).unwrap())
        .unwrap();

    assert_eq!(inspector.parse_error(), None);
    assert!(inspector.canonical_json().unwrap().contains("\"updated\""));
}

#[test]
fn canonical_json_matches_snapshot() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();

    assert_snapshot!(inspector.canonical_json().unwrap());
}

#[test]
fn json_input_round_trips_sample_bytes() {
    let bytes = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        sample_json().as_bytes(),
        InputFormat::Json,
    )
    .unwrap()
    .bytes()
    .unwrap();

    assert_eq!(bytes, sample_bytes());
}

#[test]
fn inspect_multiple_hex_lines_loads_each_message() {
    let dir = tempdir().unwrap();
    let input_path = Utf8PathBuf::from_path_buf(dir.path().join("input.hex")).unwrap();
    let mut second = serde_json::from_str::<serde_json::Value>(&sample_json()).unwrap();
    second["click"]["x"] = serde_json::Value::from(100);
    second["click"]["y"] = serde_json::Value::from(42);
    let second_bytes = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        serde_json::to_string_pretty(&second).unwrap().as_bytes(),
        InputFormat::Json,
    )
    .unwrap()
    .bytes()
    .unwrap();
    fs::write(
        &input_path,
        format!(
            "{}\n{}\n",
            hex::encode(sample_bytes()),
            hex::encode(second_bytes)
        ),
    )
    .unwrap();

    let inspectors = inspect_multiple(InspectOptions {
        schema: schema_path(),
        message: Some("SystemEvent".to_owned()),
        file: Some(input_path),
        input_format: InputFormat::Hex,
        multiple: true,
        display_options: DisplayOptions::default(),
        save_targets: SaveTargets::default(),
    })
    .unwrap();

    assert_eq!(inspectors.len(), 2);
    assert!(
        inspectors[0]
            .canonical_json()
            .unwrap()
            .contains(r#""x": 42"#)
    );
    assert!(
        inspectors[1]
            .canonical_json()
            .unwrap()
            .contains(r#""x": 100"#)
    );
}

#[test]
fn inspector_saves_all_configured_output_formats() {
    let dir = tempdir().unwrap();
    let bytes = sample_bytes();
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &bytes,
        InputFormat::Binary,
    )
    .unwrap();

    let targets = SaveTargets {
        json: Some(Utf8PathBuf::from_path_buf(dir.path().join("message.json")).unwrap()),
        base64: Some(Utf8PathBuf::from_path_buf(dir.path().join("message.base64")).unwrap()),
        hex: Some(Utf8PathBuf::from_path_buf(dir.path().join("message.hex")).unwrap()),
        binary: Some(Utf8PathBuf::from_path_buf(dir.path().join("message.bin")).unwrap()),
    };

    let saved = inspector.save(&targets).unwrap();

    assert_eq!(saved.len(), 4);
    assert_eq!(
        fs::read_to_string(targets.base64.as_ref().unwrap()).unwrap(),
        BASE64_STANDARD.encode(&bytes),
    );
    assert_eq!(
        fs::read_to_string(targets.hex.as_ref().unwrap()).unwrap(),
        hex::encode(&bytes),
    );
    assert_eq!(fs::read(targets.binary.as_ref().unwrap()).unwrap(), bytes);
    assert!(
        fs::read_to_string(targets.json.as_ref().unwrap())
            .unwrap()
            .contains("\"user clicked\"")
    );
}

#[test]
fn inspector_reports_omitted_default_enum_hint() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();

    let hint = inspector.omitted_default_enum_hint(&[
        selection::FieldPathSegment::Field("click".to_owned()),
        selection::FieldPathSegment::Field("button".to_owned()),
    ]);

    assert_eq!(
        hint.as_deref(),
        Some("Default enum Left is omitted on the wire"),
    );
}

#[test]
fn inspector_lists_and_cycles_enum_variants() {
    let mut inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();

    assert_eq!(
        inspector.enum_selection(&[
            selection::FieldPathSegment::Field("click".to_owned()),
            selection::FieldPathSegment::Field("button".to_owned()),
        ]),
        Some(EnumSelection {
            variants: vec!["Left".to_owned(), "Right".to_owned(), "Middle".to_owned()],
            current: 0,
        }),
    );

    assert_eq!(
        inspector.cycle_enum_variant(
            &[
                selection::FieldPathSegment::Field("click".to_owned()),
                selection::FieldPathSegment::Field("button".to_owned()),
            ],
            1,
        ),
        Some("Right".to_owned()),
    );
    assert!(
        inspector
            .canonical_json()
            .unwrap()
            .contains(r#""button": "Right""#)
    );
}

#[test]
fn inspector_skips_omitted_default_enum_hint_for_other_fields() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();

    assert_eq!(
        inspector.omitted_default_enum_hint(&[
            selection::FieldPathSegment::Field("click".to_owned()),
            selection::FieldPathSegment::Field("x".to_owned()),
        ]),
        None,
    );
}
