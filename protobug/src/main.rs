//! Protobuf Debugging Suite.

use std::{fs, io};

use base64::prelude::*;
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};
use derive_more::Display;
use error_stack::{IntoReportCompat as _, Result, ResultExt as _};
use protobuf::text_format;

mod line_wrap;
mod tui;

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

    Inspect {
        #[arg(long)]
        file: Utf8PathBuf,
    },
}

fn main() -> Result<(), Error> {
    let args = Args::parse();

    match args.command {
        Commands::Validate {
            schema: schema_path,
        } => {
            validate_schema(schema_path)
                .map_err(|err| err.change_context(Error::ProtobufValidate))?;
        }

        Commands::Inspect { file } => {
            inspect(file).change_context(Error::Io)?;
        }
    }

    Ok(())
}

fn inspect(file: Utf8PathBuf) -> Result<(), io::Error> {
    let file = fs::read_to_string(file)?;
    let file = decode_any_base64(&file);

    let mut tui = tui::init()?;
    let mut app = tui::App::new(file, 16);
    app.run(&mut tui)?;
    tui::restore()?;

    Ok(())
}

fn validate_schema(schema_path: Utf8PathBuf) -> Result<(), anyhow::Error> {
    let fds = protobuf_parse::Parser::new()
        .pure()
        .include({
            let mut schema_dir = schema_path.clone();
            schema_dir.pop();
            schema_dir
        })
        .input(schema_path)
        .parse_and_typecheck()
        .into_report()?;

    let tf = text_format::print_to_string_pretty(fds.file_descriptors.first().unwrap());
    println!("{tf}");

    Ok(())
}

fn decode_any_base64(encoded: &str) -> Vec<u8> {
    None.or_else(|| BASE64_STANDARD.decode(encoded).ok())
        .or_else(|| BASE64_STANDARD_NO_PAD.decode(encoded).ok())
        .or_else(|| BASE64_URL_SAFE.decode(encoded).ok())
        .or_else(|| BASE64_URL_SAFE_NO_PAD.decode(encoded).ok())
        .unwrap()
}

#[derive(Debug, Display)]
enum Error {
    Io,
    ProtobufValidate,
}

impl error_stack::Context for Error {}
