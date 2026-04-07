use std::{collections::BTreeSet, fs};

use base64::{Engine as _, prelude::BASE64_STANDARD};
use camino::Utf8PathBuf;
use error_stack::{Report, ResultExt as _};
use protobuf::{MessageDyn, reflect::MessageDescriptor, text_format};

use crate::{
    enum_edit,
    error::Inspect,
    selection::{self, FieldPath, ProtobufLine},
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
        enum_edit::omitted_default_enum_hint(&*self.data, &self.md, selected_path)
    }

    pub(crate) fn enum_selection(
        &self,
        selected_path: &[selection::FieldPathSegment],
    ) -> Option<EnumSelection> {
        enum_edit::enum_selection(&*self.data, &self.md, selected_path)
    }

    pub(crate) fn cycle_enum_variant(
        &mut self,
        selected_path: &[selection::FieldPathSegment],
        delta: isize,
    ) -> Option<String> {
        let next_variant =
            enum_edit::cycle_enum_variant(&mut *self.data, &self.md, selected_path, delta)?;
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
            return Err(Report::new(Inspect).attach(
                "No save targets were configured. Pass --save-json, --save-bin, --save-hex, or --save-base64.",
            ));
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
