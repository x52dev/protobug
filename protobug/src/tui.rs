use std::{
    io,
    time::{Duration, Instant},
};

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
    inspector::{DisplayOptions, EnumSelection, Inspector, SaveTargets},
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
    display_options: DisplayOptions,
    last_byte_pane_width: u16,
    last_status: Option<Status>,
    exit: bool,
}

#[derive(Debug, Clone)]
struct Status {
    kind: StatusKind,
    message: String,
    expires_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
enum StatusKind {
    Info,
    Error,
}

const TRANSIENT_STATUS_DURATION: Duration = Duration::from_secs(2);

impl Status {
    fn persistent(kind: StatusKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            expires_at: None,
        }
    }

    fn transient(kind: StatusKind, message: impl Into<String>, duration: Duration) -> Self {
        Self {
            kind,
            message: message.into(),
            expires_at: Some(Instant::now() + duration),
        }
    }

    fn is_expired(&self) -> bool {
        self.expires_at
            .is_some_and(|expires_at| Instant::now() >= expires_at)
    }

    fn timeout_remaining(&self) -> Option<Duration> {
        self.expires_at
            .map(|expires_at| expires_at.saturating_duration_since(Instant::now()))
    }
}

impl App<'_> {
    /// Constructs new TUI app widget.
    pub(crate) fn new(
        inspector: Inspector,
        save_targets: SaveTargets,
        display_options: DisplayOptions,
    ) -> std::result::Result<Self, error_stack::Report<Inspect>> {
        let json = inspector.canonical_json()?;

        let mut json_editor = TextArea::new(json.lines().map(ToOwned::to_owned).collect());
        json_editor.set_line_number_style(Style::default().fg(Color::DarkGray));

        Ok(Self {
            inspector,
            json_editor,
            save_targets,
            display_options,
            last_byte_pane_width: 0,
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

        let layout = Layout::horizontal(Constraint::from_fills([3, 2]));
        let left_layout = Layout::vertical(Constraint::from_fills([1, 1]));
        let [left_area, right_area] = layout.areas(main_area);
        let [top_left_area, bottom_left_area] = left_layout.areas(left_area);
        let byte_layout = Layout::horizontal(Constraint::from_fills([2, 1]));
        let [hex_area, ascii_area] = byte_layout.areas(bottom_left_area);
        let display_columns = self.effective_columns_for_pane_width(hex_area.width);
        self.last_byte_pane_width = hex_area.width;

        let json = self.current_json();
        let selected_path = self.current_selected_path(&json);
        let enum_selection = selected_path
            .as_ref()
            .and_then(|path| self.inspector.enum_selection(path));
        let omitted_default_enum_hint = selected_path
            .as_ref()
            .and_then(|path| self.inspector.omitted_default_enum_hint(path));
        let inline_hints = self.inline_hints(
            enum_selection.as_ref(),
            omitted_default_enum_hint.as_deref(),
        );
        let highlighted_bytes = selected_path
            .as_ref()
            .and_then(|path| self.inspector.highlighted_byte_indices(path).ok())
            .unwrap_or_default();
        let protobuf_text = self.protobuf_text(selected_path.as_ref());
        let protobuf_scroll = self.protobuf_scroll_offset(&protobuf_text, top_left_area.height);
        let byte_scroll =
            self.byte_scroll_offset(&highlighted_bytes, display_columns, hex_area.height);

        let para_tf = Paragraph::new(protobuf_text)
            .scroll((protobuf_scroll, 0))
            .block(
                Block::default()
                    .title("Protobuf")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Plain),
            );
        let para_hex = Paragraph::new(self.hex_text(&highlighted_bytes, display_columns))
            .block(
                Block::default()
                    .title("Hex")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Plain),
            )
            .scroll((byte_scroll, 0));
        let para_ascii = Paragraph::new(self.ascii_text(&highlighted_bytes, display_columns))
            .scroll((byte_scroll, 0))
            .block(
                Block::default()
                    .title("ASCII")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Plain),
            );

        let right_block = Block::default()
            .title("JSON")
            .borders(Borders::ALL)
            .border_type(BorderType::Plain);
        let right_inner = right_block.inner(right_area);

        frame.render_widget(para_tf, top_left_area);
        frame.render_widget(para_hex, hex_area);
        frame.render_widget(para_ascii, ascii_area);

        if !inline_hints.is_empty() {
            let [editor_area, hint_area] = Layout::vertical([
                Constraint::Min(0),
                Constraint::Length(inline_hints.len() as u16),
            ])
            .areas(right_inner);
            frame.render_widget(&self.json_editor, editor_area);
            frame.render_widget(
                Paragraph::new(inline_hints.join("\n")).style(enum_hint_style()),
                hint_area,
            );
        } else {
            frame.render_widget(&self.json_editor, right_inner);
        }
        frame.render_widget(right_block, right_area);

        let footer_block = Block::default().borders(Borders::TOP);
        frame.render_widget(
            Paragraph::new(self.status_line_for_columns(display_columns))
                .style(self.status_style()),
            footer_block.inner(footer_area),
        );
        frame.render_widget(footer_block, footer_area);
    }

    fn handle_events(&mut self) -> io::Result<()> {
        self.clear_expired_status();

        if let Some(timeout) = self.status_timeout()
            && !event::poll(timeout)?
        {
            self.clear_expired_status();
            return Ok(());
        }

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

            event::Event::Key(
                ev @ KeyEvent {
                    code: KeyCode::Char('n'),
                    ..
                },
            ) if ev.kind == event::KeyEventKind::Press
                && ev.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.cycle_selected_enum(1);
            }

            event::Event::Key(
                ev @ KeyEvent {
                    code: KeyCode::Char('p'),
                    ..
                },
            ) if ev.kind == event::KeyEventKind::Press
                && ev.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.cycle_selected_enum(-1);
            }

            event::Event::Key(
                ev @ KeyEvent {
                    code: KeyCode::Char('['),
                    ..
                },
            ) if ev.kind == event::KeyEventKind::Press => {
                self.adjust_columns(-1);
            }

            event::Event::Key(
                ev @ KeyEvent {
                    code: KeyCode::Char(']'),
                    ..
                },
            ) if ev.kind == event::KeyEventKind::Press => {
                self.adjust_columns(1);
            }

            input => {
                if self.json_editor.input(input) {
                    let json = self.json_editor.lines().join("\n");
                    if let Err(error) = self.inspector.apply_json(&json) {
                        self.last_status = Some(Status::persistent(
                            StatusKind::Error,
                            format!("Parse error: {error}"),
                        ));
                    } else if matches!(
                        self.visible_status().map(|status| status.kind),
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

                self.last_status = Some(Status::persistent(
                    StatusKind::Info,
                    format!("Saved outputs: {message}"),
                ));
            }
            Err(error) => {
                self.last_status = Some(Status::persistent(StatusKind::Error, error.to_string()));
            }
        }
    }

    #[cfg(test)]
    fn status_line(&self) -> String {
        self.status_line_for_columns(
            self.effective_columns_for_pane_width(self.last_byte_pane_width),
        )
    }

    fn status_line_for_columns(&self, columns: usize) -> String {
        if let Some(status) = self.visible_status() {
            return status.message.clone();
        }

        format!("Ctrl-C quit | Ctrl-S save | [ ] columns {}", columns,)
    }

    fn status_style(&self) -> Style {
        match self.visible_status().map(|status| status.kind) {
            Some(StatusKind::Info) => Style::default().fg(Color::Green),
            Some(StatusKind::Error) => Style::default().fg(Color::Red),
            None => Style::default().fg(Color::DarkGray),
        }
    }

    fn visible_status(&self) -> Option<&Status> {
        self.last_status
            .as_ref()
            .filter(|status| !status.is_expired())
    }

    fn clear_expired_status(&mut self) {
        if self.last_status.as_ref().is_some_and(Status::is_expired) {
            self.last_status = None;
        }
    }

    fn status_timeout(&self) -> Option<Duration> {
        self.visible_status().and_then(Status::timeout_remaining)
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

    fn protobuf_scroll_offset(&self, text: &Text<'_>, area_height: u16) -> u16 {
        let Some(line_index) = text.lines.iter().position(|line| {
            line.spans.iter().any(|span| {
                span.style.bg == Some(Color::Blue) && span.style.fg == Some(Color::White)
            })
        }) else {
            return 0;
        };

        scroll_offset_for_line(line_index, area_height)
    }

    fn hex_text(
        &self,
        highlighted_bytes: &std::collections::BTreeSet<usize>,
        columns: usize,
    ) -> Text<'static> {
        match self.inspector.bytes() {
            Ok(bytes) => Text::from(render_byte_lines(
                &bytes,
                highlighted_bytes,
                columns,
                " ",
                |byte| format!("{byte:02x}"),
            )),
            Err(error) => Text::from(error.to_string()),
        }
    }

    fn ascii_text(
        &self,
        highlighted_bytes: &std::collections::BTreeSet<usize>,
        columns: usize,
    ) -> Text<'static> {
        match self.inspector.bytes() {
            Ok(bytes) => Text::from(render_byte_lines(
                &bytes,
                highlighted_bytes,
                columns,
                "",
                |byte| match byte {
                    byte if byte.is_ascii_whitespace() => " ".to_owned(),
                    byte if byte.is_ascii_graphic() => char::from(byte).to_string(),
                    _ => ".".to_owned(),
                },
            )),
            Err(error) => Text::from(error.to_string()),
        }
    }

    fn adjust_columns(&mut self, delta: isize) {
        let current_columns = self.effective_columns_for_pane_width(self.last_byte_pane_width);
        self.display_options.columns = Some(adjust_width(current_columns, delta));
        self.last_status = Some(Status::transient(
            StatusKind::Info,
            format!(
                "Display columns set to {}",
                self.display_options.columns.unwrap_or(current_columns),
            ),
            TRANSIENT_STATUS_DURATION,
        ));
    }

    fn effective_columns_for_pane_width(&self, pane_width: u16) -> usize {
        self.display_options
            .columns
            .unwrap_or_else(|| auto_columns_for_pane_width(pane_width))
    }

    fn byte_scroll_offset(
        &self,
        highlighted_bytes: &std::collections::BTreeSet<usize>,
        columns: usize,
        area_height: u16,
    ) -> u16 {
        let Some(byte_index) = highlighted_bytes.iter().next().copied() else {
            return 0;
        };

        scroll_offset_for_line(byte_index / columns.max(1), area_height)
    }

    fn inline_hints(
        &self,
        enum_selection: Option<&EnumSelection>,
        omitted_default_enum_hint: Option<&str>,
    ) -> Vec<String> {
        let mut hints = Vec::new();

        if let Some(enum_selection) = enum_selection {
            let variants = enum_selection
                .variants
                .iter()
                .enumerate()
                .map(|(index, variant)| {
                    if index == enum_selection.current {
                        format!("[{variant}]")
                    } else {
                        variant.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            hints.push(format!("Ctrl-P/Ctrl-N enum: {variants}"));
        }

        if let Some(omitted_default_enum_hint) = omitted_default_enum_hint {
            hints.push(omitted_default_enum_hint.to_owned());
        }

        hints
    }

    fn cycle_selected_enum(&mut self, delta: isize) {
        let json = self.current_json();
        let Some(selected_path) = self.current_selected_path(&json) else {
            self.last_status = Some(Status::persistent(
                StatusKind::Info,
                "Move the cursor onto an enum value to switch variants",
            ));
            return;
        };

        let Some(variant) = self.inspector.cycle_enum_variant(&selected_path, delta) else {
            self.last_status = Some(Status::persistent(
                StatusKind::Info,
                "Move the cursor onto an enum value to switch variants",
            ));
            return;
        };

        match self.inspector.canonical_json() {
            Ok(json) => {
                let cursor = self.json_editor.cursor();
                self.json_editor
                    .set_lines(json.lines().map(ToOwned::to_owned).collect(), cursor);
                self.last_status = Some(Status::persistent(
                    StatusKind::Info,
                    format!("Enum set to {variant}"),
                ));
            }
            Err(error) => {
                self.last_status = Some(Status::persistent(StatusKind::Error, error.to_string()));
            }
        }
    }
}

const COMPACT_COLUMNS: usize = 16;
const WIDE_COLUMNS: usize = 24;

fn render_byte_lines<F>(
    bytes: &[u8],
    highlighted_bytes: &std::collections::BTreeSet<usize>,
    width: usize,
    separator: &str,
    render: F,
) -> Vec<Line<'static>>
where
    F: Fn(u8) -> String,
{
    let width = width.max(1);

    bytes
        .chunks(width)
        .enumerate()
        .map(|(chunk_index, chunk)| {
            let mut spans = Vec::new();

            for (index_in_chunk, byte) in chunk.iter().enumerate() {
                let index = chunk_index * width + index_in_chunk;
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

fn adjust_width(width: usize, delta: isize) -> usize {
    if delta >= 0 {
        width.saturating_add(delta as usize).max(1)
    } else {
        width.saturating_sub(delta.unsigned_abs()).max(1)
    }
}

fn scroll_offset_for_line(line_index: usize, area_height: u16) -> u16 {
    let visible_lines = usize::from(area_height.saturating_sub(2)).max(1);
    let top_line = line_index.saturating_sub(visible_lines.saturating_sub(1));
    top_line.min(u16::MAX as usize) as u16
}

fn auto_columns_for_pane_width(pane_width: u16) -> usize {
    if pane_width == 0 {
        return COMPACT_COLUMNS;
    }

    let inner_width = usize::from(pane_width.saturating_sub(2));

    if hex_line_width(WIDE_COLUMNS) <= inner_width {
        WIDE_COLUMNS
    } else if hex_line_width(COMPACT_COLUMNS) <= inner_width {
        COMPACT_COLUMNS
    } else {
        (1..COMPACT_COLUMNS)
            .rev()
            .find(|&columns| hex_line_width(columns) <= inner_width)
            .unwrap_or(1)
    }
}

fn hex_line_width(columns: usize) -> usize {
    columns.saturating_mul(2) + columns.saturating_sub(1)
}

fn highlight_style() -> Style {
    Style::default()
        .bg(Color::Blue)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

fn enum_hint_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::ITALIC)
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
    use crate::inspector::{DisplayOptions, InputFormat, load_inspector};

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
        let mut app =
            App::new(inspector, SaveTargets::default(), DisplayOptions::default()).unwrap();

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
        let mut app =
            App::new(inspector, SaveTargets::default(), DisplayOptions::default()).unwrap();

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
        let mut app =
            App::new(inspector, SaveTargets::default(), DisplayOptions::default()).unwrap();
        app.last_status = Some(Status::persistent(
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
            inspector,
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
        let mut app =
            App::new(inspector, SaveTargets::default(), DisplayOptions::default()).unwrap();

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
        let mut app =
            App::new(inspector, SaveTargets::default(), DisplayOptions::default()).unwrap();

        move_cursor_to(&mut app, "\"button\"");
        let json = app.current_json();
        let selected_path = app.current_selected_path(&json).unwrap();
        let omitted_default_enum_hint = app
            .inspector
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
        let mut app =
            App::new(inspector, SaveTargets::default(), DisplayOptions::default()).unwrap();

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
            inspector,
            SaveTargets::default(),
            DisplayOptions { columns: Some(4) },
        )
        .unwrap();

        move_cursor_to(&mut app, "\"y\": 100");

        let json = app.current_json();
        let selected_path = app.current_selected_path(&json).unwrap();
        let protobuf_text = app.protobuf_text(Some(&selected_path));
        let highlighted_bytes = app
            .inspector
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
            inspector,
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
        let mut app =
            App::new(inspector, SaveTargets::default(), DisplayOptions::default()).unwrap();

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
        let mut app =
            App::new(inspector, SaveTargets::default(), DisplayOptions::default()).unwrap();
        app.last_byte_pane_width = 49;

        app.adjust_columns(-8);
        assert_eq!(app.status_line(), "Display columns set to 8");

        app.last_status.as_mut().unwrap().expires_at = Some(Instant::now());

        assert_eq!(
            app.status_line(),
            "Ctrl-C quit | Ctrl-S save | [ ] columns 8"
        );

        app.clear_expired_status();
        assert!(app.last_status.is_none());
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
            .filter(|span| {
                span.style.bg == Some(Color::Blue) && span.style.fg == Some(Color::White)
            })
            .map(|span| span.content.to_string())
            .collect()
    }
}
