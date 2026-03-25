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
    line_wrap::LineWrap,
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

        let tf_repr = self.inspector.text_view();
        let hex_repr = self.inspector.hex_view();
        let ascii_repr = self.inspector.ascii_view();

        let hex_repr_wrapped = LineWrap::new(hex_repr, 16 * 3).to_string();
        let ascii_repr_wrapped = LineWrap::new(ascii_repr, 16).to_string();

        let para_tf = Paragraph::new(tf_repr).block(
            Block::default()
                .title("Protobuf")
                .borders(Borders::ALL)
                .border_type(BorderType::Plain),
        );
        let para_hex = Paragraph::new(hex_repr_wrapped).block(
            Block::default()
                .title("Hex")
                .borders(Borders::ALL)
                .border_type(BorderType::Plain),
        );
        let para_ascii = Paragraph::new(ascii_repr_wrapped).block(
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
}
