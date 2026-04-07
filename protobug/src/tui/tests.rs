use camino::Utf8PathBuf;
use insta::assert_snapshot;
use protobuf::{
    EnumOrUnknown, Message as _, MessageField, SpecialFields,
    well_known_types::timestamp::Timestamp,
};
use protogen::system_event::{
    SystemEvent,
    system_event::{Event as SystemEventVariant, MouseButton, MouseDown},
};
use ratatui::{Terminal, backend::TestBackend};
use tempfile::tempdir;
use tui_textarea::CursorMove;

use super::*;
use crate::{DisplayOptions, InputFormat, schema::load_inspector};

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

fn render_text(app: &mut App<'_>) -> String {
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.render_frame(frame)).unwrap();

    let buffer = terminal.backend().buffer();

    buffer
        .content
        .chunks(buffer.area.width as usize)
        .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn snapshot_text(app: &mut App<'_>) -> String {
    render_text(app)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}

#[test]
fn render_matches_default_layout() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();
    let mut app = App::new(
        vec![inspector],
        SaveTargets::default(),
        DisplayOptions::default(),
    )
    .unwrap();

    let rendered = snapshot_text(&mut app);

    assert_snapshot!(rendered);
}

#[test]
fn json_panel_shows_line_numbers() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();
    let mut app = App::new(
        vec![inspector],
        SaveTargets::default(),
        DisplayOptions::default(),
    )
    .unwrap();

    let rendered = render_text(&mut app);

    assert!(rendered.contains("1 {"));
    assert!(rendered.contains("4     \"x\": 42,"));
}

#[test]
fn render_matches_error_layout() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();
    let mut app = App::new(
        vec![inspector],
        SaveTargets::default(),
        DisplayOptions::default(),
    )
    .unwrap();
    app.last_status = Some(Status::new(
        StatusKind::Error,
        "Parse error: expected value",
    ));

    let rendered = snapshot_text(&mut app);

    assert_snapshot!(rendered);
}

#[test]
fn save_action_updates_status_message() {
    let dir = tempdir().unwrap();
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();
    let mut app = App::new(
        vec![inspector],
        SaveTargets {
            json: Some(Utf8PathBuf::from_path_buf(dir.path().join("message.json")).unwrap()),
            ..SaveTargets::default()
        },
        DisplayOptions::default(),
    )
    .unwrap();

    app.save_outputs();

    assert!(matches!(
        app.last_status.as_ref().map(|status| status.kind),
        Some(StatusKind::Info)
    ));
    assert!(
        app.last_status
            .as_ref()
            .unwrap()
            .message
            .contains("Saved outputs:")
    );
}

#[test]
fn render_highlights_related_panes_for_selected_json_field() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();
    let mut app = App::new(
        vec![inspector],
        SaveTargets::default(),
        DisplayOptions::default(),
    )
    .unwrap();

    move_cursor_to(&mut app, "\"seconds\"");
    let json = app.current_json();
    let selected_path = app.current_selected_path(&json);
    assert_eq!(
        selected_path,
        Some(vec![
            selection::FieldPathSegment::Field("timestamp".to_owned()),
            selection::FieldPathSegment::Field("seconds".to_owned()),
        ])
    );
    let protobuf = app.protobuf_text(selected_path.as_ref());
    assert!(protobuf.lines.iter().any(|line| {
        line.spans.iter().any(|span| {
            span.content.as_ref().contains("seconds: 1234567")
                && span.style.bg == Some(Color::Blue)
                && span.style.fg == Some(Color::White)
        })
    }));
    let highlighted_bytes = app
        .current_inspector()
        .highlighted_byte_indices(selected_path.as_ref().unwrap())
        .unwrap();
    let columns = app.effective_columns_for_pane_width(48);
    let hex = app.hex_text(&highlighted_bytes, columns);
    let ascii = app.ascii_text(&highlighted_bytes, columns);

    let highlighted_hex = highlighted_span_contents(&hex);
    assert!(
        highlighted_hex
            .windows(4)
            .any(|window| window == ["08", "87", "ad", "4b"])
    );

    let highlighted_ascii = highlighted_span_contents(&ascii);
    assert_eq!(highlighted_ascii.len(), 4);
    assert!(highlighted_ascii.contains(&"K".to_owned()));
}

#[test]
fn render_shows_hint_for_omitted_default_enum_variant() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();
    let mut app = App::new(
        vec![inspector],
        SaveTargets::default(),
        DisplayOptions::default(),
    )
    .unwrap();

    move_cursor_to(&mut app, "\"button\"");
    let json = app.current_json();
    let selected_path = app.current_selected_path(&json).unwrap();
    let omitted_default_enum_hint = app
        .current_inspector()
        .omitted_default_enum_hint(&selected_path)
        .unwrap();

    let rendered = render_text(&mut app);

    assert!(rendered.contains("Ctrl-P/Ctrl-N enum:"));
    assert!(rendered.contains("[Left]"));
    assert!(rendered.contains("Right"));
    assert_eq!(
        omitted_default_enum_hint,
        "Default enum Left is omitted on the wire"
    );
}

#[test]
fn cycling_selected_enum_updates_json_and_status() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();
    let mut app = App::new(
        vec![inspector],
        SaveTargets::default(),
        DisplayOptions::default(),
    )
    .unwrap();

    move_cursor_to(&mut app, "\"button\"");
    app.cycle_selected_enum(1);

    assert!(app.current_json().contains(r#""button": "Right""#));
    assert_eq!(app.status_line(), "Enum set to Right");

    let rendered = render_text(&mut app);
    assert!(rendered.contains("Ctrl-P/Ctrl-N enum:"));
    assert!(rendered.contains("Left"));
    assert!(rendered.contains("[Right]"));
}

#[test]
fn selected_content_scrolls_into_view() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();
    let mut app = App::new(
        vec![inspector],
        SaveTargets::default(),
        DisplayOptions { columns: Some(4) },
    )
    .unwrap();

    move_cursor_to(&mut app, "\"y\": 100");

    let json = app.current_json();
    let selected_path = app.current_selected_path(&json).unwrap();
    let protobuf_text = app.protobuf_text(Some(&selected_path));
    let highlighted_bytes = app
        .current_inspector()
        .highlighted_byte_indices(&selected_path)
        .unwrap();

    assert!(app.protobuf_scroll_offset(&protobuf_text, 4) > 0);
    assert!(app.byte_scroll_offset(&highlighted_bytes, 4, 4) > 0);
}

#[test]
fn render_respects_shared_display_columns() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();
    let app = App::new(
        vec![inspector],
        SaveTargets::default(),
        DisplayOptions { columns: Some(8) },
    )
    .unwrap();

    let hex = app.hex_text(&Default::default(), 8);
    let ascii = app.ascii_text(&Default::default(), 8);

    assert_eq!(hex.lines.len(), 4);
    assert_eq!(ascii.lines.len(), 4);
}

#[test]
fn adjusting_columns_updates_status_message() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();
    let mut app = App::new(
        vec![inspector],
        SaveTargets::default(),
        DisplayOptions::default(),
    )
    .unwrap();

    app.adjust_columns(-8);
    assert_eq!(app.display_options.columns, Some(8));
    assert_eq!(app.status_line(), "Display columns set to 8");

    app.adjust_columns(4);
    assert_eq!(app.display_options.columns, Some(12));
    assert_eq!(app.status_line(), "Display columns set to 12");
}

#[test]
fn expired_column_status_returns_to_footer_help() {
    let inspector = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();
    let mut app = App::new(
        vec![inspector],
        SaveTargets::default(),
        DisplayOptions::default(),
    )
    .unwrap();
    app.last_byte_pane_width = 49;

    app.adjust_columns(-8);
    assert_eq!(app.status_line(), "Display columns set to 8");

    app.last_status.as_mut().unwrap().expires_at = Instant::now();

    assert_eq!(
        app.status_line(),
        "Ctrl-C quit | Ctrl-S save | [ ] columns 8"
    );

    app.clear_expired_status();
    assert!(app.last_status.is_none());
}

#[test]
fn navigating_messages_switches_visible_payload() {
    let first = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        &sample_bytes(),
        InputFormat::Binary,
    )
    .unwrap();
    let mut second_json =
        serde_json::from_str::<serde_json::Value>(&first.canonical_json().unwrap()).unwrap();
    second_json["click"]["x"] = serde_json::Value::from(100);
    second_json["click"]["y"] = serde_json::Value::from(42);
    let second = load_inspector(
        schema_path().as_ref(),
        Some("SystemEvent"),
        serde_json::to_string_pretty(&second_json)
            .unwrap()
            .as_bytes(),
        InputFormat::Json,
    )
    .unwrap();
    let mut app = App::new(
        vec![first, second],
        SaveTargets::default(),
        DisplayOptions::default(),
    )
    .unwrap();

    assert!(app.current_json().contains(r#""x": 42"#));
    assert_eq!(app.message_suffix(), " (1/2)");
    assert_eq!(
        app.status_line(),
        "Ctrl-C quit | Ctrl-S save | Ctrl-J/K line 1/2 | [ ] columns 16"
    );

    app.navigate_message(1);

    assert!(app.current_json().contains(r#""x": 100"#));
    assert_eq!(app.message_suffix(), " (2/2)");
    assert_eq!(app.status_line(), "Message 2 of 2");

    app.last_status = None;
    assert_eq!(
        app.status_line(),
        "Ctrl-C quit | Ctrl-S save | Ctrl-J/K line 2/2 | [ ] columns 16"
    );
}

#[test]
fn auto_columns_expand_when_pane_is_wide_enough() {
    assert_eq!(auto_columns_for_pane_width(73), 24);
    assert_eq!(auto_columns_for_pane_width(72), 16);
    assert_eq!(auto_columns_for_pane_width(25), 8);
}

#[test]
fn scroll_offset_keeps_highlighted_line_visible() {
    assert_eq!(scroll_offset_for_line(0, 4), 0);
    assert_eq!(scroll_offset_for_line(3, 4), 2);
    assert_eq!(scroll_offset_for_line(5, 5), 3);
}

fn move_cursor_to(app: &mut App<'_>, needle: &str) {
    let (row, col) = app
        .json_editor
        .lines()
        .iter()
        .enumerate()
        .find_map(|(row, line)| line.find(needle).map(|col| (row, col)))
        .unwrap();

    app.json_editor
        .move_cursor(CursorMove::Jump(row as u16, col as u16));
}

fn highlighted_span_contents(text: &Text<'_>) -> Vec<String> {
    text.lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .filter(|span| span.style.bg == Some(Color::Blue) && span.style.fg == Some(Color::White))
        .map(|span| span.content.to_string())
        .collect()
}
