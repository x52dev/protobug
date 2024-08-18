use std::{fmt::Write as _, io};

use crossterm::{
    event::{self, KeyCode, KeyEvent, KeyModifiers},
    execute, terminal,
};
use protobuf::{reflect::MessageDescriptor, text_format};
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, Paragraph},
    Terminal,
};
use tui_textarea::TextArea;

use crate::line_wrap::LineWrap;

pub type Tui = Terminal<CrosstermBackend<io::Stdout>>;

/// Sets up terminal for TUI display.
pub(crate) fn init() -> io::Result<Tui> {
    execute!(io::stdout(), terminal::EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;
    Terminal::new(CrosstermBackend::new(io::stdout()))
}

/// Restores terminal to original state..
pub(crate) fn restore() -> io::Result<()> {
    execute!(io::stdout(), terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

pub(crate) struct App<'a> {
    md: MessageDescriptor,
    data: Box<dyn protobuf::MessageDyn>,
    json_editor: TextArea<'a>,
    exit: bool,
}

impl App<'_> {
    /// Constructs new TUI app widget.
    pub(crate) fn new(md: MessageDescriptor, data: Box<dyn protobuf::MessageDyn>) -> Self {
        let json = protobuf_json_mapping::print_to_string_with_options(
            &*data,
            &protobuf_json_mapping::PrintOptions {
                enum_values_int: false,
                proto_field_name: false,
                always_output_default_values: true,
                ..Default::default()
            },
        )
        .unwrap();
        let json = serde_json::to_string_pretty(
            &serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        )
        .unwrap();

        let json_editor = TextArea::new(json.lines().map(ToOwned::to_owned).collect());

        Self {
            md,
            data,
            json_editor,
            exit: false,
        }
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
        let layout = Layout::horizontal(Constraint::from_fills([1, 1]));
        let left_layout = Layout::vertical(Constraint::from_fills([1, 1, 1]));
        let [left_area, right_area] = layout.areas(frame.area());
        let [top_left_area, middle_left_area, bottom_left_area] = left_layout.areas(left_area);

        let tf_repr = text_format::print_to_string_pretty(&*self.data);

        let bytes = self.data.write_to_bytes_dyn().unwrap();

        let hex_repr = bytes.iter().fold(String::new(), |mut buf, byte| {
            write!(buf, "{byte:02x} ").expect("Formatting to strings should always be possible");
            buf
        });

        let hex_repr_wrapped = LineWrap::new(hex_repr, 16 * 3).to_string();

        let ascii_repr = bytes.iter().fold(String::new(), |mut buf, byte| {
            let preview = match byte {
                byte if byte.is_ascii_whitespace() => ' ',
                byte if byte.is_ascii_graphic() => char::from(*byte),
                _ => '.',
            };

            debug_assert_eq!(preview.len_utf8(), 1, "{preview} is not a single byte");

            write!(buf, "{preview}").expect("Formatting to strings should always be possible");
            buf
        });

        let ascii_repr_wrapped = LineWrap::new(ascii_repr, 16).to_string();

        let para_tf = Paragraph::new(tf_repr);
        let para_hex = Paragraph::new(hex_repr_wrapped);
        let para_ascii = Paragraph::new(ascii_repr_wrapped);

        let right_block = Block::default()
            .borders(Borders::LEFT)
            .border_type(BorderType::Plain);

        frame.render_widget(para_tf, top_left_area);
        frame.render_widget(para_hex, middle_left_area);
        frame.render_widget(para_ascii, bottom_left_area);

        frame.render_widget(&self.json_editor, right_block.inner(right_area));
        frame.render_widget(right_block, right_area);
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

            input => {
                self.json_editor.input(input);
                let json = self.json_editor.lines().join(" ");
                if let Ok(msg) = protobuf_json_mapping::parse_dyn_from_str(&self.md, &json) {
                    self.data = msg;
                }
            }
        };

        Ok(())
    }
}
