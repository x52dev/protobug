use std::{
    fs,
    io::{self, Read as _},
};

use base64::prelude::*;
use camino::Utf8Path;
use error_stack::{Report, ResultExt as _};

use crate::{error::Inspect, message::InputFormat};

pub(crate) fn read_input(path: Option<&Utf8Path>) -> std::result::Result<Vec<u8>, Report<Inspect>> {
    match path {
        None => Err(Report::new(Inspect).attach(
            "No input file was provided. Pass --file <path> or --file - to read from stdin.",
        )),
        Some(path) if path.as_str() == "-" => read_stdin(),
        Some(path) => fs::read(path)
            .attach_with(|| format!("Input file: {path}"))
            .change_context(Inspect),
    }
}

fn read_stdin() -> std::result::Result<Vec<u8>, Report<Inspect>> {
    let mut buf = Vec::new();

    io::stdin()
        .read_to_end(&mut buf)
        .attach("Input source: stdin")
        .change_context(Inspect)?;

    Ok(buf)
}

pub(crate) fn json_input_as_text(raw_input: &[u8]) -> std::result::Result<&str, Report<Inspect>> {
    std::str::from_utf8(raw_input)
        .attach("Input format: json")
        .change_context(Inspect)
}

pub(crate) fn resolve_edit_input_format(
    raw_input: &[u8],
    requested: InputFormat,
    multiple: bool,
) -> std::result::Result<InputFormat, Report<Inspect>> {
    Ok(match requested {
        InputFormat::Auto => {
            if multiple {
                return Err(Report::new(Inspect)
                    .attach("`edit --multiple` requires `--input-format hex` or `base64`"));
            }
            if let Ok(json) = json_input_as_text(raw_input)
                && serde_json::from_str::<serde_json::Value>(json).is_ok()
            {
                InputFormat::Json
            } else if decode_hex(raw_input).is_ok() {
                InputFormat::Hex
            } else if decode_base64(raw_input).is_ok() {
                InputFormat::Base64
            } else {
                InputFormat::Binary
            }
        }
        other => other,
    })
}

pub(crate) fn validate_multiple_input_format(
    input_format: InputFormat,
) -> std::result::Result<(), Report<Inspect>> {
    match input_format {
        InputFormat::Base64 | InputFormat::Hex => Ok(()),
        InputFormat::Auto => Err(Report::new(Inspect)
            .attach("`edit --multiple` requires `--input-format hex` or `base64`")),
        other => Err(Report::new(Inspect).attach(format!(
            "`edit --multiple` only supports `hex` or `base64` input, got `{}`",
            other.as_str()
        ))),
    }
}

pub(crate) fn decode_input(
    raw_input: &[u8],
    input_format: InputFormat,
) -> std::result::Result<Vec<u8>, Report<Inspect>> {
    match input_format {
        InputFormat::Json => Err(Report::new(Inspect)
            .attach("Input format: json must be parsed through the JSON message loader")),
        InputFormat::Binary => Ok(raw_input.to_vec()),
        InputFormat::Base64 => decode_base64(raw_input),
        InputFormat::Hex => decode_hex(raw_input),
        InputFormat::Auto => {
            if let Ok(text) = std::str::from_utf8(raw_input) {
                let trimmed = text.trim();

                if looks_like_hex(trimmed)
                    && let Ok(decoded) = decode_hex(raw_input)
                {
                    return Ok(decoded);
                }

                if looks_like_base64(trimmed)
                    && let Ok(decoded) = decode_base64(raw_input)
                {
                    return Ok(decoded);
                }
            }

            Ok(raw_input.to_vec())
        }
    }
}

pub(crate) fn decode_base64(raw_input: &[u8]) -> std::result::Result<Vec<u8>, Report<Inspect>> {
    let text = input_as_text(raw_input, InputFormat::Base64)?;
    let compact = strip_ascii_whitespace(text);

    BASE64_STANDARD
        .decode(&compact)
        .or_else(|_| BASE64_STANDARD_NO_PAD.decode(&compact))
        .or_else(|_| BASE64_URL_SAFE.decode(&compact))
        .or_else(|_| BASE64_URL_SAFE_NO_PAD.decode(&compact))
        .attach("Input format: base64")
        .change_context(Inspect)
}

pub(crate) fn decode_hex(raw_input: &[u8]) -> std::result::Result<Vec<u8>, Report<Inspect>> {
    let text = input_as_text(raw_input, InputFormat::Hex)?;
    let compact = strip_ascii_whitespace(text);

    hex::decode(compact)
        .attach("Input format: hex")
        .change_context(Inspect)
}

fn input_as_text(
    raw_input: &[u8],
    format: InputFormat,
) -> std::result::Result<&str, Report<Inspect>> {
    std::str::from_utf8(raw_input)
        .attach_with(|| format!("Input format: {}", format.as_str()))
        .change_context(Inspect)
}

fn strip_ascii_whitespace(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect()
}

fn looks_like_hex(text: &str) -> bool {
    let compact = strip_ascii_whitespace(text);
    !compact.is_empty()
        && compact.len().is_multiple_of(2)
        && compact.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn looks_like_base64(text: &str) -> bool {
    let compact = strip_ascii_whitespace(text);
    !compact.is_empty()
        && compact.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_')
        })
}
