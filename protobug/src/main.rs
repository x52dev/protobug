//! Protobuf Debugging Suite.

#![allow(dead_code)]

use std::fs;

use base64::prelude::*;
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};
use derive_more::derive::{Display, Error};
use error::Inspect;
use error_stack::{IntoReportCompat as _, Result, ResultExt as _};
use protobuf::{reflect::FileDescriptor, text_format};

mod error;
mod line_wrap;
mod tui;

use self::error::{InvalidSchema, MultipleTopLevelMessages, NoTopLevelMessages};

#[derive(Debug, Parser)]
#[clap(version, about, rename_all = "kebab-case")]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
#[command(rename_all = "kebab-case")]
enum Commands {
    /// Validates a protobuf schema.
    Validate {
        #[arg(long)]
        schema: Utf8PathBuf,
    },

    /// Inspects a protobuf file (base64) using a schema.
    Inspect {
        #[arg(long)]
        schema: Utf8PathBuf,

        #[arg(long)]
        file: Utf8PathBuf,
    },
}

#[derive(Debug, Display, Error)]
#[display("Exit")]
pub(crate) struct ProtobugError;

fn main() -> Result<(), ProtobugError> {
    let args = Args::parse();

    match args.command {
        Commands::Validate {
            schema: schema_path,
        } => {
            validate_schema(schema_path).map_err(|err| err.change_context(ProtobugError))?;
        }

        Commands::Inspect { schema, file } => {
            inspect(schema, file).change_context(ProtobugError)?;
        }
    }

    Ok(())
}

fn inspect(schema: Utf8PathBuf, file: Utf8PathBuf) -> Result<(), Inspect> {
    let mut file_descriptor_protos = protobuf_parse::Parser::new()
        .pure()
        .includes(schema.parent().as_slice())
        .input(&schema)
        .parse_and_typecheck()
        .unwrap()
        .file_descriptors;

    let fd_proto = file_descriptor_protos.pop().unwrap();
    let deps = file_descriptor_protos
        .into_iter()
        .map(|fd_proto| FileDescriptor::new_dynamic(fd_proto, &[]).unwrap())
        .collect::<Vec<_>>();
    let fd = FileDescriptor::new_dynamic(fd_proto, &deps).change_context(Inspect)?;

    let msg_name = single_msg_name(&fd)
        .attach_printable(format!("Schema file: {schema}"))
        .change_context(Inspect)?;

    // TODO: provide choice when there are multiple top-level types

    let md = fd.message_by_package_relative_name(&msg_name).unwrap();

    let file_contents = fs::read_to_string(&file)
        .attach_printable_lazy(|| format!("File: {file}"))
        .change_context(Inspect)?;

    let decoded_message = decode_any_base64(file_contents.trim_end())
        .attach_printable_lazy(|| format!("File: {file}"))
        .change_context(Inspect)?;

    let msg = md
        .parse_from_bytes(&decoded_message)
        .attach_printable_lazy(|| format!("File: {file}"))
        .change_context(Inspect)?;

    let mut tui = tui::init().change_context(Inspect)?;
    let mut app = tui::App::new(md, msg);
    app.run(&mut tui).change_context(Inspect)?;
    tui::restore().change_context(Inspect)?;

    Ok(())
}

/// Returns name of single top-level message in schema.
///
/// # Errors
///
/// Returns error if there are more or less than 1 top-level message.
fn single_msg_name(fd: &FileDescriptor) -> Result<String, InvalidSchema> {
    let mut messages = fd.messages();

    let md = messages
        .next()
        .ok_or(NoTopLevelMessages)
        .change_context(InvalidSchema)?;

    let more = messages.count();

    if more == 0 {
        Ok(md.name().to_owned())
    } else {
        Err(MultipleTopLevelMessages)
            .attach_printable(format!("Top-level messages found: {}", more + 1))
            .change_context(InvalidSchema)
    }
}

fn validate_schema(schema_path: Utf8PathBuf) -> Result<(), anyhow::Error> {
    let fds = protobuf_parse::Parser::new()
        .pure()
        .includes(schema_path.parent().as_slice())
        .input(schema_path)
        .parse_and_typecheck()
        .into_report()?;

    let tf = text_format::print_to_string_pretty(fds.file_descriptors.first().unwrap());
    println!("{tf}");

    Ok(())
}

fn decode_any_base64(encoded: &str) -> Result<Vec<u8>, base64::DecodeError> {
    Ok(BASE64_STANDARD
        .decode(encoded)
        .or_else(|_| BASE64_STANDARD_NO_PAD.decode(encoded))
        .or_else(|_| BASE64_URL_SAFE.decode(encoded))
        .or_else(|_| BASE64_URL_SAFE_NO_PAD.decode(encoded))?)
}
