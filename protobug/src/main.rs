//! Protobuf Debugging Suite.

use camino::Utf8PathBuf;
use clap::{Parser, Subcommand, ValueEnum};
use derive_more::derive::{Display, Error};
use error_stack::{Report, ResultExt as _};
use protobug::{
    DisplayOptions, InputFormat, InspectOptions, SaveTargets, run_inspect, validate_schema,
};

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

    /// Inspects a protobuf payload using a schema.
    Inspect {
        #[arg(long)]
        schema: Utf8PathBuf,

        /// Message name relative to the package in the schema.
        #[arg(long)]
        message: Option<String>,

        /// Input file path. Reads from stdin when omitted or set to "-".
        #[arg(long)]
        file: Option<Utf8PathBuf>,

        /// How to decode the input payload before parsing.
        #[arg(long, value_enum, default_value_t = InputFormatArg::Auto)]
        input_format: InputFormatArg,

        /// Bytes per row shared by the hex and ASCII panes.
        #[arg(long, default_value_t = 16, value_parser = parse_width)]
        columns: usize,

        /// Save the current message as pretty JSON when Ctrl-S is pressed.
        #[arg(long)]
        save_json: Option<Utf8PathBuf>,

        /// Save the current message as raw protobuf bytes when Ctrl-S is pressed.
        #[arg(long)]
        save_bin: Option<Utf8PathBuf>,

        /// Save the current message as hex when Ctrl-S is pressed.
        #[arg(long)]
        save_hex: Option<Utf8PathBuf>,

        /// Save the current message as base64 when Ctrl-S is pressed.
        #[arg(long)]
        save_base64: Option<Utf8PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InputFormatArg {
    Auto,
    Base64,
    Hex,
    Binary,
}

impl From<InputFormatArg> for InputFormat {
    fn from(value: InputFormatArg) -> Self {
        match value {
            InputFormatArg::Auto => Self::Auto,
            InputFormatArg::Base64 => Self::Base64,
            InputFormatArg::Hex => Self::Hex,
            InputFormatArg::Binary => Self::Binary,
        }
    }
}

fn parse_width(value: &str) -> Result<usize, String> {
    let width = value.parse::<usize>().map_err(|error| error.to_string())?;

    if width == 0 {
        return Err("width must be at least 1".to_owned());
    }

    Ok(width)
}

#[derive(Debug, Display, Error)]
#[display("Exit")]
pub(crate) struct ProtobugError;

fn main() -> std::result::Result<(), Report<ProtobugError>> {
    let args = Args::parse();

    match args.command {
        Commands::Validate {
            schema: schema_path,
        } => {
            println!(
                "{}",
                validate_schema(schema_path).map_err(|err| err.change_context(ProtobugError))?,
            );
        }

        Commands::Inspect {
            schema,
            message,
            file,
            input_format,
            columns,
            save_json,
            save_bin,
            save_hex,
            save_base64,
        } => {
            run_inspect(InspectOptions {
                schema,
                message,
                file,
                input_format: input_format.into(),
                display_options: DisplayOptions { columns },
                save_targets: SaveTargets {
                    json: save_json,
                    base64: save_base64,
                    hex: save_hex,
                    binary: save_bin,
                },
            })
            .change_context(ProtobugError)?;
        }
    }

    Ok(())
}
