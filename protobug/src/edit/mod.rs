#[cfg(test)]
mod tests;

use base64::{Engine as _, prelude::BASE64_STANDARD};
use camino::Utf8PathBuf;
use error_stack::{Report, ResultExt as _};

use crate::{
    decode,
    error::Inspect,
    json,
    message::{InputFormat, Inspector},
    schema::load_inspector,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditOptions {
    pub schema: Utf8PathBuf,
    pub message: Option<String>,
    pub file: Option<Utf8PathBuf>,
    pub input_format: InputFormat,
    pub filter: Option<String>,
    pub multiple: bool,
}

struct EditedMessage {
    inspector: Inspector,
    source_format: InputFormat,
}

pub fn edit_to_json(options: EditOptions) -> std::result::Result<String, Report<Inspect>> {
    edit(options)?.inspector.canonical_json()
}

pub fn edit_to_bytes(options: EditOptions) -> std::result::Result<Vec<u8>, Report<Inspect>> {
    edit(options)?.inspector.bytes()
}

pub fn edit_to_json_lines(options: EditOptions) -> std::result::Result<String, Report<Inspect>> {
    let (edited, had_trailing_newline) = edit_multiple(options)?;
    let lines = edited
        .into_iter()
        .map(|edited| json::compact_json(&edited.inspector))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(json::join_lines(lines, had_trailing_newline))
}

pub fn edit_to_encoded_lines(
    options: EditOptions,
    output_format: InputFormat,
) -> std::result::Result<String, Report<Inspect>> {
    let (edited, had_trailing_newline) = edit_multiple(options)?;
    let lines = edited
        .into_iter()
        .map(|edited| json::encode_line_output(&edited.inspector, output_format))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(json::join_lines(lines, had_trailing_newline))
}

pub fn edit_in_place(options: EditOptions) -> std::result::Result<(), Report<Inspect>> {
    let path = options
        .file
        .clone()
        .ok_or_else(|| Report::new(Inspect).attach("`edit --in-place` requires `--file <path>`"))?;

    if path.as_str() == "-" {
        return Err(Report::new(Inspect)
            .attach("`edit --in-place` does not support stdin; pass a file path instead"));
    }

    if options.multiple {
        let output_format = match options.input_format {
            InputFormat::Base64 => InputFormat::Base64,
            InputFormat::Hex => InputFormat::Hex,
            InputFormat::Auto => {
                return Err(Report::new(Inspect).attach(
                    "`edit --multiple --in-place` requires `--input-format hex` or `base64`",
                ));
            }
            other => {
                return Err(Report::new(Inspect).attach(format!(
                    "`edit --multiple` only supports `hex` or `base64` input, got `{}`",
                    other.as_str()
                )));
            }
        };
        let output = edit_to_encoded_lines(options, output_format)?;
        return std::fs::write(&path, output)
            .attach_with(|| format!("Output file: {path}"))
            .change_context(Inspect);
    }

    let edited = edit(options)?;
    let bytes = match edited.source_format {
        InputFormat::Json => edited.inspector.canonical_json()?.into_bytes(),
        InputFormat::Base64 => BASE64_STANDARD
            .encode(edited.inspector.bytes()?)
            .into_bytes(),
        InputFormat::Hex => hex::encode(edited.inspector.bytes()?).into_bytes(),
        InputFormat::Binary => edited.inspector.bytes()?,
        InputFormat::Auto => unreachable!("edit input format is resolved before serialization"),
    };

    std::fs::write(&path, bytes)
        .attach_with(|| format!("Output file: {path}"))
        .change_context(Inspect)
}

fn edit(options: EditOptions) -> std::result::Result<EditedMessage, Report<Inspect>> {
    if options.multiple {
        return Err(Report::new(Inspect).attach(
            "`edit` expected a single payload; use the line-based edit helpers for `--multiple`",
        ));
    }

    let input = decode::read_input(options.file.as_deref())?;
    let source_format = decode::resolve_edit_input_format(&input, options.input_format, false)?;
    let mut inspector = load_inspector(
        options.schema.as_ref(),
        options.message.as_deref(),
        &input,
        source_format,
    )?;

    if let Some(filter) = options.filter.as_deref() {
        let filtered = json::apply_json_filter(&inspector.canonical_json()?, filter)?;
        inspector
            .apply_json(&filtered)
            .map_err(|error| Report::new(Inspect).attach(error))?;
    }

    Ok(EditedMessage {
        inspector,
        source_format,
    })
}

fn edit_multiple(
    options: EditOptions,
) -> std::result::Result<(Vec<EditedMessage>, bool), Report<Inspect>> {
    decode::validate_multiple_input_format(options.input_format)?;
    let input = decode::read_input(options.file.as_deref())?;
    let text = std::str::from_utf8(&input)
        .attach("Input format: line-based text")
        .change_context(Inspect)?;
    let had_trailing_newline = text.ends_with('\n');
    let source_format = decode::resolve_edit_input_format(&input, options.input_format, true)?;
    let mut edited = Vec::new();

    for line in text.lines() {
        let line = line.trim_end_matches('\r');

        if line.is_empty() {
            return Err(Report::new(Inspect)
                .attach("`edit --multiple` does not support empty lines in the input file"));
        }
        let mut inspector = load_inspector(
            options.schema.as_ref(),
            options.message.as_deref(),
            line.as_bytes(),
            source_format,
        )?;

        if let Some(filter) = options.filter.as_deref() {
            let filtered = json::apply_json_filter(&inspector.canonical_json()?, filter)?;
            inspector
                .apply_json(&filtered)
                .map_err(|error| Report::new(Inspect).attach(error))?;
        }

        edited.push(EditedMessage {
            inspector,
            source_format,
        });
    }

    Ok((edited, had_trailing_newline))
}
