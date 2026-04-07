use ratatui::prelude::*;

pub(super) const COMPACT_COLUMNS: usize = 16;
pub(super) const WIDE_COLUMNS: usize = 24;

pub(super) fn render_byte_lines<F>(
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

pub(super) fn adjust_width(width: usize, delta: isize) -> usize {
    if delta >= 0 {
        width.saturating_add(delta as usize).max(1)
    } else {
        width.saturating_sub(delta.unsigned_abs()).max(1)
    }
}

pub(super) fn scroll_offset_for_line(line_index: usize, area_height: u16) -> u16 {
    let visible_lines = usize::from(area_height.saturating_sub(2)).max(1);
    let top_line = line_index.saturating_sub(visible_lines.saturating_sub(1));
    top_line.min(u16::MAX as usize) as u16
}

pub(super) fn auto_columns_for_pane_width(pane_width: u16) -> usize {
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

pub(super) fn highlight_style() -> Style {
    Style::default()
        .bg(Color::Blue)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

pub(super) fn enum_hint_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::ITALIC)
}
