use std::io;

use crossterm::{
    event::{self, KeyCode, KeyEvent, KeyModifiers},
    execute, terminal,
};
use ratatui::{
    Terminal,
    prelude::*,
    widgets::{Block, BorderType, Borders, Paragraph},
};
use tui_textarea::TextArea;

use crate::{
    error::Inspect,
    inspector::{Inspector, SaveTargets},
    selection::{self, FieldPath},
};

pub type Tui = Terminal<CrosstermBackend<io::Stdout>>;

pub(crate) struct Session {
    terminal: Tui,
}

impl Session {
    /// Sets up terminal for TUI display.
    pub(crate) fn new() -> io::Result<Self> {
        execute!(io::stdout(), terminal::EnterAlternateScreen)?;
        terminal::enable_raw_mode()?;

        Ok(Self {
            terminal: Terminal::new(CrosstermBackend::new(io::stdout()))?,
        })
    }

    pub(crate) fn terminal_mut(&mut self) -> &mut Tui {
        &mut self.terminal
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), terminal::LeaveAlternateScreen);
    }
}

pub(crate) struct App<'a> {
    inspector: Inspector,
    json_editor: TextArea<'a>,
    save_targets: SaveTargets,
    last_status: Option<Status>,
    exit: bool,
}

#[derive(Debug, Clone)]
struct Status {
    kind: StatusKind,
    message: String,
}

#[derive(Debug, Clone, Copy)]
enum StatusKind {
    Info,
    Error,
}

impl App<'_> {
    /// Constructs new TUI app widget.
    pub(crate) fn new(
        inspector: Inspector,
        save_targets: SaveTargets,
    ) -> std::result::Result<Self, error_stack::Report<Inspect>> {
        let json = inspector.canonical_json()?;

        let json_editor = TextArea::new(json.lines().map(ToOwned::to_owned).collect());

        Ok(Self {
            inspector,
            json_editor,
            save_targets,
            last_status: None,
            exit: false,
        })
    }

    /// Runs main execution loop for the TUI app.
    pub(crate) fn run(&mut self, tui: &mut Tui) -> io::Result<()> {
        while !self.exit {
            tui.draw(|frame| self.render_frame(frame))?;
            self.handle_events()?;
        }

        Ok(())
    }

    fn render_frame(&mut self, frame: &mut Frame<'_>) {
        let root_layout = Layout::vertical([Constraint::Min(0), Constraint::Length(2)]);
        let [main_area, footer_area] = root_layout.areas(frame.area());

        let layout = Layout::horizontal(Constraint::from_fills([1, 1]));
        let left_layout = Layout::vertical(Constraint::from_fills([1, 1, 1]));
        let [left_area, right_area] = layout.areas(main_area);
        let [top_left_area, middle_left_area, bottom_left_area] = left_layout.areas(left_area);

        let json = self.current_json();
        let selected_path = self.current_selected_path(&json);
        let highlighted_bytes = selected_path
            .as_ref()
            .and_then(|path| self.inspector.highlighted_byte_indices(path).ok())
            .unwrap_or_default();

        let para_tf = Paragraph::new(self.protobuf_text(selected_path.as_ref())).block(
            Block::default()
                .title("Protobuf")
                .borders(Borders::ALL)
                .border_type(BorderType::Plain),
        );
        let para_hex = Paragraph::new(self.hex_text(&highlighted_bytes)).block(
            Block::default()
                .title("Hex")
                .borders(Borders::ALL)
                .border_type(BorderType::Plain),
        );
        let para_ascii = Paragraph::new(self.ascii_text(&highlighted_bytes)).block(
            Block::default()
                .title("ASCII")
                .borders(Borders::ALL)
                .border_type(BorderType::Plain),
        );

        let right_block = Block::default()
            .title("JSON")
            .borders(Borders::ALL)
            .border_type(BorderType::Plain);

        frame.render_widget(para_tf, top_left_area);
        frame.render_widget(para_hex, middle_left_area);
        frame.render_widget(para_ascii, bottom_left_area);

        frame.render_widget(&self.json_editor, right_block.inner(right_area));
        frame.render_widget(right_block, right_area);

        let footer_block = Block::default().borders(Borders::TOP);
        frame.render_widget(
            Paragraph::new(self.status_line()).style(self.status_style()),
            footer_block.inner(footer_area),
        );
        frame.render_widget(footer_block, footer_area);
    }

    fn handle_events(&mut self) -> io::Result<()> {
        match event::read()? {
            // check that the event is a key press event as crossterm also emits
            // key release and repeat events on Windows
            event::Event::Key(
                ev @ KeyEvent {
                    code: KeyCode::Char('c'),
                    ..
                },
            ) if ev.kind == event::KeyEventKind::Press
                && ev.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.exit = true;
            }

            event::Event::Key(
                ev @ KeyEvent {
                    code: KeyCode::Char('s'),
                    ..
                },
            ) if ev.kind == event::KeyEventKind::Press
                && ev.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.save_outputs();
            }

            input => {
                if self.json_editor.input(input) {
                    let json = self.json_editor.lines().join("\n");
                    if let Err(error) = self.inspector.apply_json(&json) {
                        self.last_status = Some(Status {
                            kind: StatusKind::Error,
                            message: format!("Parse error: {error}"),
                        });
                    } else if matches!(
                        self.last_status.as_ref().map(|status| status.kind),
                        Some(StatusKind::Error)
                    ) {
                        self.last_status = None;
                    }
                }
            }
        };

        Ok(())
    }

    fn save_outputs(&mut self) {
        match self.inspector.save(&self.save_targets) {
            Ok(paths) => {
                let message = paths
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");

                self.last_status = Some(Status {
                    kind: StatusKind::Info,
                    message: format!("Saved outputs: {message}"),
                });
            }
            Err(error) => {
                self.last_status = Some(Status {
                    kind: StatusKind::Error,
                    message: error.to_string(),
                });
            }
        }
    }

    fn status_line(&self) -> String {
        if let Some(status) = &self.last_status {
            return status.message.clone();
        }

        "Ctrl-C quit | Ctrl-S save configured outputs".to_owned()
    }

    fn status_style(&self) -> Style {
        match self.last_status.as_ref().map(|status| status.kind) {
            Some(StatusKind::Info) => Style::default().fg(Color::Green),
            Some(StatusKind::Error) => Style::default().fg(Color::Red),
            None => Style::default().fg(Color::DarkGray),
        }
    }

    fn current_json(&self) -> String {
        self.json_editor.lines().join("\n")
    }

    fn current_selected_path(&self, json: &str) -> Option<FieldPath> {
        if self.inspector.parse_error().is_some() {
            return None;
        }

        self.inspector
            .selected_path_for_json_cursor(json, self.json_editor.cursor())
    }

    fn protobuf_text(&self, selected_path: Option<&FieldPath>) -> Text<'static> {
        let lines = self
            .inspector
            .protobuf_lines()
            .into_iter()
            .map(|line| {
                let style = selected_path
                    .filter(|selected| selection::related_path(selected, &line.path))
                    .map_or_else(Style::default, |_| highlight_style());

                Line::from(vec![Span::styled(line.text, style)])
            })
            .collect::<Vec<_>>();

        Text::from(lines)
    }

    fn hex_text(&self, highlighted_bytes: &std::collections::BTreeSet<usize>) -> Text<'static> {
        match self.inspector.bytes() {
            Ok(bytes) => Text::from(render_byte_lines(&bytes, highlighted_bytes, " ", |byte| {
                format!("{byte:02x}")
            })),
            Err(error) => Text::from(error.to_string()),
        }
    }

    fn ascii_text(&self, highlighted_bytes: &std::collections::BTreeSet<usize>) -> Text<'static> {
        match self.inspector.bytes() {
            Ok(bytes) => {
                Text::from(render_byte_lines(
                    &bytes,
                    highlighted_bytes,
                    "",
                    |byte| match byte {
                        byte if byte.is_ascii_whitespace() => " ".to_owned(),
                        byte if byte.is_ascii_graphic() => char::from(byte).to_string(),
                        _ => ".".to_owned(),
                    },
                ))
            }
            Err(error) => Text::from(error.to_string()),
        }
    }
}

fn render_byte_lines<F>(
    bytes: &[u8],
    highlighted_bytes: &std::collections::BTreeSet<usize>,
    separator: &str,
    render: F,
) -> Vec<Line<'static>>
where
    F: Fn(u8) -> String,
{
    bytes
        .chunks(16)
        .enumerate()
        .map(|(chunk_index, chunk)| {
            let mut spans = Vec::new();

            for (index_in_chunk, byte) in chunk.iter().enumerate() {
                let index = chunk_index * 16 + index_in_chunk;
                let style = if highlighted_bytes.contains(&index) {
                    highlight_style()
                } else {
                    Style::default()
                };

                spans.push(Span::styled(render(*byte), style));

                if index_in_chunk + 1 < chunk.len() {
                    spans.push(Span::raw(separator.to_owned()));
                }
            }

            Line::from(spans)
        })
        .collect()
}

fn highlight_style() -> Style {
    Style::default()
        .bg(Color::Blue)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
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
    use crate::inspector::{InputFormat, load_inspector};

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
        let mut app = App::new(inspector, SaveTargets::default()).unwrap();

        let rendered = snapshot_text(&mut app);

        assert_snapshot!(rendered);
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
        let mut app = App::new(inspector, SaveTargets::default()).unwrap();
        app.last_status = Some(Status {
            kind: StatusKind::Error,
            message: "Parse error: expected value".to_owned(),
        });

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
            inspector,
            SaveTargets {
                json: Some(Utf8PathBuf::from_path_buf(dir.path().join("message.json")).unwrap()),
                ..SaveTargets::default()
            },
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
        let mut app = App::new(inspector, SaveTargets::default()).unwrap();

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
            .inspector
            .highlighted_byte_indices(selected_path.as_ref().unwrap())
            .unwrap();
        let hex = app.hex_text(&highlighted_bytes);
        let ascii = app.ascii_text(&highlighted_bytes);

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
            .filter(|span| {
                span.style.bg == Some(Color::Blue) && span.style.fg == Some(Color::White)
            })
            .map(|span| span.content.to_string())
            .collect()
    }
}
