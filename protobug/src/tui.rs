use std::{fmt::Write as _, io};

use crossterm::{
    event::{self, KeyCode, KeyModifiers},
    terminal,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
    Terminal,
};

use crate::line_wrap::LineWrap;

pub type Tui = Terminal<CrosstermBackend<io::Stdout>>;

/// Sets up terminal for TUI display.
pub(crate) fn init() -> io::Result<Tui> {
    // execute!(io::stdout(), terminal::EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;
    Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(24),
        },
    )
}

/// Restores terminal to original state..
pub(crate) fn restore() -> io::Result<()> {
    // execute!(io::stdout(), terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

pub(crate) struct App {
    data: Vec<u8>,
    wrap_at: usize,
    exit: bool,
}

impl App {
    /// Constructs new TUI app widget.
    pub(crate) fn new(data: Vec<u8>, wrap_at: usize) -> Self {
        Self {
            data,
            wrap_at,
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
        let layout = Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]);
        let [left_area, right_area] = layout.areas(frame.size());

        let hex_repr = self.data.iter().fold(String::new(), |mut buf, byte| {
            write!(buf, "{byte:02x} ").expect("Formatting to strings should always be possible");
            buf
        });

        let hex_repr_wrapped = LineWrap::new(hex_repr, self.wrap_at * 3).to_string();

        let ascii_repr = self.data.iter().fold(String::new(), |mut buf, byte| {
            let preview = match byte {
                byte if byte.is_ascii_whitespace() => ' ',
                byte if byte.is_ascii_graphic() => char::from(*byte),
                _ => '.',
            };

            debug_assert_eq!(preview.len_utf8(), 1, "{preview} is not a single byte");

            write!(buf, "{preview}").expect("Formatting to strings should always be possible");
            buf
        });

        let ascii_repr_wrapped = LineWrap::new(ascii_repr, self.wrap_at).to_string();

        // assert_eq!(
        //     hex_lines, ascii_lines,
        //     "Hex and ASCII outputs differ in size",
        // );

        let left_para = Paragraph::new(hex_repr_wrapped);
        let right_para = Paragraph::new(ascii_repr_wrapped);

        let right_block = Block::default()
            .borders(Borders::LEFT)
            .border_type(ratatui::widgets::BorderType::Plain);

        frame.render_widget(left_para, left_area);
        frame.render_widget(right_para, right_block.inner(right_area));
        frame.render_widget(right_block, right_area);
    }

    fn handle_events(&mut self) -> io::Result<()> {
        match event::read()? {
            // check that the event is a key press event as crossterm also emits
            // key release and repeat events on Windows
            event::Event::Key(ev) if ev.kind == event::KeyEventKind::Press => {
                self.handle_key_event(ev);
            }

            _ => {}
        };

        Ok(())
    }

    fn handle_key_event(&mut self, ev: event::KeyEvent) {
        match ev.code {
            KeyCode::Right => self.wrap_at += 1,
            KeyCode::Left => self.wrap_at -= 1,

            // exit (q or ctrl-c)
            KeyCode::Char('q') => self.exit = true,
            KeyCode::Char('c') if ev.modifiers.contains(KeyModifiers::CONTROL) => self.exit = true,

            _ => {}
        }
    }
}
