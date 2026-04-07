mod render;
#[cfg(test)]
mod tests;

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
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use tui_textarea::TextArea;

use self::render::{
    adjust_width, auto_columns_for_pane_width, enum_hint_style, highlight_style, render_byte_lines,
    scroll_offset_for_line,
};
use crate::{
    error::Inspect,
    message::{DisplayOptions, EnumSelection, Inspector, SaveTargets},
    selection::{self, FieldPath},
};

pub type Tui = Terminal<CrosstermBackend<io::Stdout>>;

pub(crate) struct Session {
    terminal: Tui,
}

impl Session {
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
    inspectors: Vec<Inspector>,
    current_index: usize,
    json_editor: TextArea<'a>,
    message_selector: Option<String>,
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
    expires_at: Instant,
}

#[derive(Debug, Clone, Copy)]
enum StatusKind {
    Info,
    Error,
}

const STATUS_DURATION: Duration = Duration::from_secs(4);

impl Status {
    fn new(kind: StatusKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            expires_at: Instant::now() + STATUS_DURATION,
        }
    }

    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    fn timeout_remaining(&self) -> Duration {
        self.expires_at.saturating_duration_since(Instant::now())
    }
}

impl App<'_> {
    pub(crate) fn new(
        inspectors: Vec<Inspector>,
        save_targets: SaveTargets,
        display_options: DisplayOptions,
    ) -> std::result::Result<Self, error_stack::Report<Inspect>> {
        let json = inspectors
            .first()
            .ok_or_else(|| error_stack::Report::new(Inspect).attach("No inspectors were loaded"))?
            .canonical_json()?;

        let mut json_editor = TextArea::new(json.lines().map(ToOwned::to_owned).collect());
        json_editor.set_line_number_style(Style::default().fg(Color::DarkGray));

        Ok(Self {
            inspectors,
            current_index: 0,
            json_editor,
            message_selector: None,
            save_targets,
            display_options,
            last_byte_pane_width: 0,
            last_status: None,
            exit: false,
        })
    }

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
        let [left_area, right_area] = layout.areas(main_area);
        let (top_left_area, bottom_left_area) =
            if self.display_options.show_hex || self.display_options.show_ascii {
                let left_layout = Layout::vertical(Constraint::from_fills([1, 1]));
                let [top_left_area, bottom_left_area] = left_layout.areas(left_area);
                (top_left_area, Some(bottom_left_area))
            } else {
                (left_area, None)
            };
        let byte_pane_width = bottom_left_area.map_or(0, |area| self.byte_pane_width(area.width));
        let display_columns = self.effective_columns_for_pane_width(byte_pane_width);
        self.last_byte_pane_width = byte_pane_width;

        let json = self.current_json();
        let selected_path = self.current_selected_path(&json);
        let enum_selection = selected_path
            .as_ref()
            .and_then(|path| self.current_inspector().enum_selection(path));
        let omitted_default_enum_hint = selected_path
            .as_ref()
            .and_then(|path| self.current_inspector().omitted_default_enum_hint(path));
        let inline_hints = self.inline_hints(
            enum_selection.as_ref(),
            omitted_default_enum_hint.as_deref(),
        );
        let highlighted_bytes = selected_path
            .as_ref()
            .and_then(|path| self.current_inspector().highlighted_byte_indices(path).ok())
            .unwrap_or_default();
        let protobuf_text = self.protobuf_text(selected_path.as_ref());
        let protobuf_scroll = self.protobuf_scroll_offset(&protobuf_text, top_left_area.height);
        let byte_scroll = bottom_left_area.map_or(0, |area| {
            self.byte_scroll_offset(&highlighted_bytes, display_columns, area.height)
        });

        let para_tf = Paragraph::new(protobuf_text)
            .scroll((protobuf_scroll, 0))
            .block(
                Block::default()
                    .title("Protobuf")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Plain),
            );
        let right_block = Block::default()
            .title(format!("JSON{}", self.message_suffix()))
            .borders(Borders::ALL)
            .border_type(BorderType::Plain);
        let right_inner = right_block.inner(right_area);

        frame.render_widget(para_tf, top_left_area);
        if let Some(bottom_left_area) = bottom_left_area {
            self.render_byte_panes(
                frame,
                bottom_left_area,
                &highlighted_bytes,
                display_columns,
                byte_scroll,
            );
        }

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

        if let Some(selector) = &self.message_selector {
            self.render_message_selector(frame, selector);
        }

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
            event::Event::Key(ev) if self.message_selector.is_some() => {
                self.handle_message_selector_key(ev);
            }

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
                    code: KeyCode::Char('a'),
                    ..
                },
            ) if ev.kind == event::KeyEventKind::Press
                && ev.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.toggle_ascii_pane();
            }

            event::Event::Key(
                ev @ KeyEvent {
                    code: KeyCode::Char('g'),
                    ..
                },
            ) if ev.kind == event::KeyEventKind::Press
                && ev.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.open_message_selector();
            }

            event::Event::Key(
                ev @ KeyEvent {
                    code: KeyCode::Char('j'),
                    ..
                },
            ) if ev.kind == event::KeyEventKind::Press
                && ev.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.navigate_message(1);
            }

            event::Event::Key(
                ev @ KeyEvent {
                    code: KeyCode::Char('k'),
                    ..
                },
            ) if ev.kind == event::KeyEventKind::Press
                && ev.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.navigate_message(-1);
            }

            event::Event::Key(
                ev @ KeyEvent {
                    code: KeyCode::Char('x'),
                    ..
                },
            ) if ev.kind == event::KeyEventKind::Press
                && ev.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.toggle_hex_pane();
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
                    if let Err(error) = self.current_inspector_mut().apply_json(&json) {
                        self.show_error(format!("Parse error: {error}"));
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
        match self.current_inspector().save(&self.save_targets) {
            Ok(paths) => {
                let message = paths
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");

                self.show_info(format!("Saved outputs: {message}"));
            }
            Err(error) => self.show_error(error.to_string()),
        }
    }

    fn render_byte_panes(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        highlighted_bytes: &std::collections::BTreeSet<usize>,
        columns: usize,
        byte_scroll: u16,
    ) {
        match (
            self.display_options.show_hex,
            self.display_options.show_ascii,
        ) {
            (true, true) => {
                let byte_layout = Layout::horizontal(Constraint::from_fills([2, 1]));
                let [hex_area, ascii_area] = byte_layout.areas(area);
                frame.render_widget(
                    self.hex_paragraph(highlighted_bytes, columns, byte_scroll),
                    hex_area,
                );
                frame.render_widget(
                    self.ascii_paragraph(highlighted_bytes, columns, byte_scroll),
                    ascii_area,
                );
            }
            (true, false) => frame.render_widget(
                self.hex_paragraph(highlighted_bytes, columns, byte_scroll),
                area,
            ),
            (false, true) => frame.render_widget(
                self.ascii_paragraph(highlighted_bytes, columns, byte_scroll),
                area,
            ),
            (false, false) => {}
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

        let message_help = if self.inspectors.len() > 1 {
            format!(
                " | Ctrl-G picker | Ctrl-J/K msg {}/{}",
                self.current_index + 1,
                self.inspectors.len()
            )
        } else {
            String::new()
        };

        format!(
            "Ctrl-C quit | Ctrl-S save{message_help} | Ctrl-X hex | Ctrl-A ascii | [ ] columns {columns}"
        )
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
        self.visible_status().map(Status::timeout_remaining)
    }

    fn show_status(&mut self, kind: StatusKind, message: impl Into<String>) {
        self.last_status = Some(Status::new(kind, message));
    }

    fn show_info(&mut self, message: impl Into<String>) {
        self.show_status(StatusKind::Info, message);
    }

    fn show_error(&mut self, message: impl Into<String>) {
        self.show_status(StatusKind::Error, message);
    }

    fn render_message_selector(&self, frame: &mut Frame<'_>, selector: &str) {
        let help_line = self.message_selector_help_line();
        let jump_line = self.message_selector_jump_line();
        let overlay_area = centered_rect(
            self.message_selector_width(selector, &help_line, &jump_line),
            6,
            frame.area(),
        );
        let block = Block::default()
            .title("Go To Message")
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .style(Style::default().bg(Color::Black).fg(Color::White));
        let inner = block.inner(overlay_area);
        let lines = vec![
            Line::from(format!(
                "Enter message number (1-{})",
                self.inspectors.len()
            )),
            Line::from(format!("> {selector}")),
            help_line,
            jump_line,
        ];

        frame.render_widget(Clear, overlay_area);
        frame.render_widget(Paragraph::new(lines).block(block), overlay_area);
        frame.set_cursor_position(Position::new(
            inner.x + 2 + selector.chars().count() as u16,
            inner.y + 1,
        ));
    }

    fn message_selector_help_line(&self) -> Line<'static> {
        let label_style = Style::default().fg(Color::DarkGray);

        Line::from(vec![
            Span::styled(
                "Enter",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": jump  ", label_style),
            Span::styled(
                "Esc",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(": cancel", label_style),
        ])
    }

    fn message_selector_jump_line(&self) -> Line<'static> {
        let label_style = Style::default().fg(Color::DarkGray);

        Line::from(vec![
            Span::styled(
                "Ctrl-B",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": first message  ", label_style),
            Span::styled(
                "Ctrl-E",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": last message", label_style),
        ])
    }

    fn message_selector_width(
        &self,
        selector: &str,
        help_line: &Line<'_>,
        jump_line: &Line<'_>,
    ) -> u16 {
        let prompt_width = format!("Enter message number (1-{})", self.inspectors.len())
            .chars()
            .count();
        let input_width = format!("> {selector}").chars().count();
        let help_width = help_line.width();
        let jump_width = jump_line.width();
        let content_width = prompt_width
            .max(input_width)
            .max(help_width)
            .max(jump_width);

        (content_width as u16).saturating_add(4)
    }

    fn open_message_selector(&mut self) {
        if self.inspectors.len() <= 1 {
            self.show_info("Only one message is loaded");
            return;
        }

        self.message_selector = Some((self.current_index + 1).to_string());
        self.last_status = None;
    }

    fn handle_message_selector_key(&mut self, ev: KeyEvent) {
        if ev.kind != event::KeyEventKind::Press {
            return;
        }

        match ev.code {
            KeyCode::Esc => {
                self.message_selector = None;
                self.show_info("Jump cancelled");
            }
            KeyCode::Enter => self.submit_message_selector(),
            KeyCode::Home | KeyCode::Char('b') if ev.modifiers.contains(KeyModifiers::CONTROL) => {
                self.set_current_message(0);
            }
            KeyCode::End | KeyCode::Char('e') if ev.modifiers.contains(KeyModifiers::CONTROL) => {
                self.set_current_message(self.inspectors.len().saturating_sub(1));
            }
            KeyCode::Backspace => {
                if let Some(selector) = &mut self.message_selector {
                    selector.pop();
                }
            }
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                if let Some(selector) = &mut self.message_selector {
                    selector.push(ch);
                }
            }
            _ => {}
        }
    }

    fn submit_message_selector(&mut self) {
        let Some(selector) = self.message_selector.take() else {
            return;
        };

        let Ok(message_number) = selector.parse::<usize>() else {
            self.show_error("Enter a valid message number");
            return;
        };

        if !(1..=self.inspectors.len()).contains(&message_number) {
            self.show_error(format!(
                "Message number must be between 1 and {}",
                self.inspectors.len()
            ));
            return;
        }

        self.set_current_message(message_number - 1);
    }

    fn current_json(&self) -> String {
        self.json_editor.lines().join("\n")
    }

    fn hex_paragraph(
        &self,
        highlighted_bytes: &std::collections::BTreeSet<usize>,
        columns: usize,
        byte_scroll: u16,
    ) -> Paragraph<'static> {
        Paragraph::new(self.hex_text(highlighted_bytes, columns))
            .block(
                Block::default()
                    .title("Hex")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Plain),
            )
            .scroll((byte_scroll, 0))
    }

    fn ascii_paragraph(
        &self,
        highlighted_bytes: &std::collections::BTreeSet<usize>,
        columns: usize,
        byte_scroll: u16,
    ) -> Paragraph<'static> {
        Paragraph::new(self.ascii_text(highlighted_bytes, columns))
            .scroll((byte_scroll, 0))
            .block(
                Block::default()
                    .title("ASCII")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Plain),
            )
    }

    fn current_selected_path(&self, json: &str) -> Option<FieldPath> {
        if self.current_inspector().parse_error().is_some() {
            return None;
        }

        self.current_inspector()
            .selected_path_for_json_cursor(json, self.json_editor.cursor())
    }

    fn protobuf_text(&self, selected_path: Option<&FieldPath>) -> Text<'static> {
        let lines = self
            .current_inspector()
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
        match self.current_inspector().bytes() {
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
        match self.current_inspector().bytes() {
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
        self.show_info(format!(
            "Display columns set to {}",
            self.display_options.columns.unwrap_or(current_columns),
        ));
    }

    fn byte_pane_width(&self, bottom_left_width: u16) -> u16 {
        match (
            self.display_options.show_hex,
            self.display_options.show_ascii,
        ) {
            (true, true) => {
                let byte_layout = Layout::horizontal(Constraint::from_fills([2, 1]));
                let [hex_area, _] = byte_layout.areas(Rect::new(0, 0, bottom_left_width, 1));
                hex_area.width
            }
            (true, false) | (false, true) => bottom_left_width,
            (false, false) => 0,
        }
    }

    fn toggle_ascii_pane(&mut self) {
        self.display_options.show_ascii = !self.display_options.show_ascii;
        self.show_info(if self.display_options.show_ascii {
            "ASCII pane shown"
        } else {
            "ASCII pane hidden"
        });
    }

    fn toggle_hex_pane(&mut self) {
        self.display_options.show_hex = !self.display_options.show_hex;
        self.show_info(if self.display_options.show_hex {
            "Hex pane shown"
        } else {
            "Hex pane hidden"
        });
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
            self.show_info("Move the cursor onto an enum value to switch variants");
            return;
        };

        let Some(variant) = self
            .current_inspector_mut()
            .cycle_enum_variant(&selected_path, delta)
        else {
            self.show_info("Move the cursor onto an enum value to switch variants");
            return;
        };

        match self.current_inspector().canonical_json() {
            Ok(json) => {
                let cursor = self.json_editor.cursor();
                self.json_editor
                    .set_lines(json.lines().map(ToOwned::to_owned).collect(), cursor);
                self.show_info(format!("Enum set to {variant}"));
            }
            Err(error) => self.show_error(error.to_string()),
        }
    }

    fn current_inspector(&self) -> &Inspector {
        &self.inspectors[self.current_index]
    }

    fn current_inspector_mut(&mut self) -> &mut Inspector {
        &mut self.inspectors[self.current_index]
    }

    fn message_suffix(&self) -> String {
        if self.inspectors.len() > 1 {
            format!(" ({}/{})", self.current_index + 1, self.inspectors.len())
        } else {
            String::new()
        }
    }

    fn navigate_message(&mut self, delta: isize) {
        if self.inspectors.len() <= 1 {
            self.show_info("Only one message is loaded");
            return;
        }

        let current = self.current_index as isize;
        let last = self.inspectors.len().saturating_sub(1) as isize;
        let next = (current + delta).clamp(0, last) as usize;

        if next == self.current_index {
            self.show_info(format!(
                "Message {} of {}",
                self.current_index + 1,
                self.inspectors.len()
            ));
            return;
        }

        self.set_current_message(next);
    }

    fn set_current_message(&mut self, index: usize) {
        self.current_index = index;
        self.message_selector = None;
        match self.current_inspector().canonical_json() {
            Ok(json) => {
                self.json_editor
                    .set_lines(json.lines().map(ToOwned::to_owned).collect(), (0, 0));
                self.last_status = None;
                self.show_info(format!(
                    "Message {} of {}",
                    self.current_index + 1,
                    self.inspectors.len()
                ));
            }
            Err(error) => self.show_error(error.to_string()),
        }
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let popup_width = width.min(area.width.saturating_sub(2)).max(1);
    let popup_height = height.min(area.height.saturating_sub(2)).max(1);
    let x = area.x + area.width.saturating_sub(popup_width) / 2;
    let y = area.y + area.height.saturating_sub(popup_height) / 2;

    Rect::new(x, y, popup_width, popup_height)
}
