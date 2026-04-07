mod decode;
mod enums;
mod json;
mod schema;
#[cfg(test)]
mod tests;

use std::{collections::BTreeSet, fs};

use base64::prelude::*;
use camino::Utf8PathBuf;
use error_stack::{Report, ResultExt as _};
use protobuf::{MessageDyn, reflect::MessageDescriptor, text_format};

use crate::{
    error::Inspect,
    selection::{self, FieldPath, ProtobufLine},
    tui,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InputFormat {
    #[default]
    Auto,
    Json,
    Base64,
    Hex,
    Binary,
}

impl InputFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Json => "json",
            Self::Base64 => "base64",
            Self::Hex => "hex",
            Self::Binary => "binary",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveTargets {
    pub json: Option<Utf8PathBuf>,
    pub base64: Option<Utf8PathBuf>,
    pub hex: Option<Utf8PathBuf>,
    pub binary: Option<Utf8PathBuf>,
}

impl SaveTargets {
    pub fn is_empty(&self) -> bool {
        self.json.is_none() && self.base64.is_none() && self.hex.is_none() && self.binary.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectOptions {
    pub schema: Utf8PathBuf,
    pub message: Option<String>,
    pub file: Option<Utf8PathBuf>,
    pub input_format: InputFormat,
    pub multiple: bool,
    pub display_options: DisplayOptions,
    pub save_targets: SaveTargets,
}

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplayOptions {
    pub columns: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnumSelection {
    pub(crate) variants: Vec<String>,
    pub(crate) current: usize,
}

pub struct Inspector {
    md: MessageDescriptor,
    data: Box<dyn MessageDyn>,
    parse_error: Option<String>,
}

impl Inspector {
    pub fn new(md: MessageDescriptor, data: Box<dyn MessageDyn>) -> Self {
        Self {
            md,
            data,
            parse_error: None,
        }
    }

    pub fn apply_json(&mut self, json: &str) -> Result<(), String> {
        match protobuf_json_mapping::parse_dyn_from_str(&self.md, json) {
            Ok(msg) => {
                self.data = msg;
                self.parse_error = None;
                Ok(())
            }
            Err(err) => {
                let error = err.to_string();
                self.parse_error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub fn parse_error(&self) -> Option<&str> {
        self.parse_error.as_deref()
    }

    pub fn canonical_json(&self) -> std::result::Result<String, Report<Inspect>> {
        let json = protobuf_json_mapping::print_to_string_with_options(
            &*self.data,
            &protobuf_json_mapping::PrintOptions {
                enum_values_int: false,
                proto_field_name: false,
                always_output_default_values: true,
                ..Default::default()
            },
        )
        .change_context(Inspect)?;

        let value = serde_json::from_str::<serde_json::Value>(&json).change_context(Inspect)?;
        serde_json::to_string_pretty(&value).change_context(Inspect)
    }

    pub fn text_view(&self) -> String {
        text_format::print_to_string_pretty(&*self.data)
    }

    pub(crate) fn protobuf_lines(&self) -> Vec<ProtobufLine> {
        selection::protobuf_lines(&self.md, &*self.data)
    }

    pub fn bytes(&self) -> std::result::Result<Vec<u8>, Report<Inspect>> {
        self.data.write_to_bytes_dyn().change_context(Inspect)
    }

    pub(crate) fn selected_path_for_json_cursor(
        &self,
        json: &str,
        cursor: (usize, usize),
    ) -> Option<FieldPath> {
        selection::selected_path_for_json_cursor(&self.md, json, cursor)
    }

    pub(crate) fn highlighted_byte_indices(
        &self,
        selected_path: &[selection::FieldPathSegment],
    ) -> std::result::Result<BTreeSet<usize>, Report<Inspect>> {
        let bytes = self.bytes()?;
        let mut highlighted = selection::highlighted_byte_indices(&self.md, &bytes, selected_path);

        if highlighted.is_empty()
            && matches!(
                selected_path.last(),
                Some(selection::FieldPathSegment::Index(_))
            )
        {
            highlighted = selection::highlighted_byte_indices(
                &self.md,
                &bytes,
                &selected_path[..selected_path.len().saturating_sub(1)],
            );
        }

        Ok(highlighted)
    }

    pub(crate) fn omitted_default_enum_hint(
        &self,
        selected_path: &[selection::FieldPathSegment],
    ) -> Option<String> {
        enums::omitted_default_enum_hint(&*self.data, &self.md, selected_path)
    }

    pub(crate) fn enum_selection(
        &self,
        selected_path: &[selection::FieldPathSegment],
    ) -> Option<EnumSelection> {
        enums::enum_selection(&*self.data, &self.md, selected_path)
    }

    pub(crate) fn cycle_enum_variant(
        &mut self,
        selected_path: &[selection::FieldPathSegment],
        delta: isize,
    ) -> Option<String> {
        let next_variant =
            enums::cycle_enum_variant(&mut *self.data, &self.md, selected_path, delta)?;
        self.parse_error = None;
        Some(next_variant)
    }

    pub fn hex_view(&self) -> String {
        match self.bytes() {
            Ok(bytes) => bytes
                .iter()
                .fold(String::new(), |mut buf, byte| {
                    use std::fmt::Write as _;
                    write!(buf, "{byte:02x} ")
                        .expect("formatting bytes into a string should always succeed");
                    buf
                })
                .trim_end()
                .to_owned(),
            Err(err) => format!("<serialization error: {err}>"),
        }
    }

    pub fn ascii_view(&self) -> String {
        match self.bytes() {
            Ok(bytes) => bytes.iter().fold(String::new(), |mut buf, byte| {
                use std::fmt::Write as _;
                let preview = match byte {
                    byte if byte.is_ascii_whitespace() => ' ',
                    byte if byte.is_ascii_graphic() => char::from(*byte),
                    _ => '.',
                };

                write!(buf, "{preview}")
                    .expect("formatting bytes into a string should always succeed");
                buf
            }),
            Err(err) => format!("<serialization error: {err}>"),
        }
    }

    pub fn save(
        &self,
        targets: &SaveTargets,
    ) -> std::result::Result<Vec<Utf8PathBuf>, Report<Inspect>> {
        if targets.is_empty() {
            return Err(Report::new(Inspect)
                .attach("No save targets were configured. Pass --save-json, --save-bin, --save-hex, or --save-base64."));
        }

        let bytes = self.bytes()?;
        let mut saved = Vec::new();

        if let Some(path) = &targets.binary {
            fs::write(path, &bytes)
                .attach_with(|| format!("Output file: {path}"))
                .change_context(Inspect)?;
            saved.push(path.clone());
        }

        if let Some(path) = &targets.hex {
            fs::write(path, hex::encode(&bytes))
                .attach_with(|| format!("Output file: {path}"))
                .change_context(Inspect)?;
            saved.push(path.clone());
        }

        if let Some(path) = &targets.base64 {
            fs::write(path, BASE64_STANDARD.encode(&bytes))
                .attach_with(|| format!("Output file: {path}"))
                .change_context(Inspect)?;
            saved.push(path.clone());
        }

        if let Some(path) = &targets.json {
            fs::write(path, self.canonical_json()?)
                .attach_with(|| format!("Output file: {path}"))
                .change_context(Inspect)?;
            saved.push(path.clone());
        }

        Ok(saved)
    }
}

pub fn run_inspect(options: InspectOptions) -> std::result::Result<(), Report<Inspect>> {
    let save_targets = options.save_targets.clone();
    let display_options = options.display_options;
    let inspectors = inspect(options)?;

    let mut terminal = tui::Session::new().change_context(Inspect)?;
    let mut app =
        tui::App::new(inspectors, save_targets, display_options).change_context(Inspect)?;
    app.run(terminal.terminal_mut()).change_context(Inspect)?;

    Ok(())
}

pub fn inspect_to_json(options: InspectOptions) -> std::result::Result<String, Report<Inspect>> {
    inspect_one(options)?.canonical_json()
}

pub fn inspect_to_bytes(options: InspectOptions) -> std::result::Result<Vec<u8>, Report<Inspect>> {
    inspect_one(options)?.bytes()
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
        return fs::write(&path, output)
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

    fs::write(&path, bytes)
        .attach_with(|| format!("Output file: {path}"))
        .change_context(Inspect)
}

pub fn validate_schema(
    schema_path: Utf8PathBuf,
) -> std::result::Result<String, Report<anyhow::Error>> {
    schema::validate_schema(schema_path)
}

pub fn load_inspector(
    schema: &camino::Utf8Path,
    message: Option<&str>,
    raw_input: &[u8],
    input_format: InputFormat,
) -> std::result::Result<Inspector, Report<Inspect>> {
    schema::load_inspector(schema, message, raw_input, input_format)
}

pub fn available_message_names(fd: &protobuf::reflect::FileDescriptor) -> Vec<String> {
    schema::available_message_names(fd)
}

fn inspect(options: InspectOptions) -> std::result::Result<Vec<Inspector>, Report<Inspect>> {
    if options.multiple {
        return inspect_multiple(options);
    }

    Ok(vec![inspect_one(options)?])
}

fn inspect_one(options: InspectOptions) -> std::result::Result<Inspector, Report<Inspect>> {
    if options.multiple {
        return Err(Report::new(Inspect).attach(
            "expected a single payload; multiple payloads require the multi-message inspector path",
        ));
    }

    let input = decode::read_input(options.file.as_deref())?;
    load_inspector(
        options.schema.as_ref(),
        options.message.as_deref(),
        &input,
        options.input_format,
    )
}

fn inspect_multiple(
    options: InspectOptions,
) -> std::result::Result<Vec<Inspector>, Report<Inspect>> {
    decode::validate_multiple_input_format(options.input_format)?;
    let input = decode::read_input(options.file.as_deref())?;
    let text = std::str::from_utf8(&input)
        .attach("Input format: line-based text")
        .change_context(Inspect)?;
    let mut inspectors = Vec::new();

    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            return Err(Report::new(Inspect)
                .attach("`inspect --multiple` does not support empty lines in the input file"));
        }

        inspectors.push(load_inspector(
            options.schema.as_ref(),
            options.message.as_deref(),
            line.as_bytes(),
            options.input_format,
        )?);
    }

    if inspectors.is_empty() {
        return Err(Report::new(Inspect)
            .attach("`inspect --multiple` did not find any payload lines in the input file"));
    }

    Ok(inspectors)
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
