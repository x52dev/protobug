//! Protobuf Debugging Suite.

use std::io::Write as _;

use base64::Engine as _;
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand, ValueEnum};
use derive_more::derive::{Display, Error};
use error_stack::{Report, ResultExt as _};
use protobug::{
    DisplayOptions, EditOptions, InputFormat, InspectOptions, SaveTargets, edit_to_bytes,
    edit_to_json, inspect_to_bytes, inspect_to_json, run_inspect, validate_schema,
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

        /// Input file path. Pass "-" to read from stdin.
        #[arg(long)]
        file: Option<Utf8PathBuf>,

        /// How to decode the input payload before parsing.
        #[arg(long, value_enum, default_value_t = InputFormatArg::Auto)]
        input_format: InputFormatArg,

        /// Bytes per row shared by the hex and ASCII panes.
        #[arg(long, value_parser = parse_width)]
        columns: Option<usize>,

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

        /// Print the decoded message in the selected format and exit.
        #[arg(
            long,
            value_enum,
            conflicts_with_all = ["columns", "save_json", "save_bin", "save_hex", "save_base64"]
        )]
        print_format: Option<OutputFormatArg>,
    },

    /// Edits a protobuf message starting from JSON input.
    Edit {
        #[arg(long)]
        schema: Utf8PathBuf,

        /// Message name relative to the package in the schema.
        #[arg(long)]
        message: Option<String>,

        /// Input JSON file path. Pass "-" to read from stdin.
        #[arg(long)]
        file: Option<Utf8PathBuf>,

        /// Jaq filter to run against the input JSON before protobuf encoding.
        #[arg(long)]
        filter: Option<String>,

        /// Print the edited message in the selected format.
        #[arg(long, value_enum, default_value_t = OutputFormatArg::Binary)]
        print_format: OutputFormatArg,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InputFormatArg {
    Auto,
    Base64,
    Hex,
    Binary,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormatArg {
    Json,
    Binary,
    Base64,
    Hex,
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

fn write_output(bytes: &[u8], output_format: OutputFormatArg) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();

    match output_format {
        OutputFormatArg::Json => {
            unreachable!("json output is handled before raw output is written")
        }
        OutputFormatArg::Binary => stdout.write_all(bytes)?,
        OutputFormatArg::Base64 => {
            writeln!(stdout, "{}", base64::prelude::BASE64_STANDARD.encode(bytes))?
        }
        OutputFormatArg::Hex => writeln!(stdout, "{}", hex::encode(bytes))?,
    }

    Ok(())
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
            print_format,
        } => {
            let options = InspectOptions {
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
            };

            match print_format {
                Some(OutputFormatArg::Json) => {
                    println!(
                        "{}",
                        inspect_to_json(options).change_context(ProtobugError)?,
                    );
                }
                Some(output_format) => {
                    let bytes = inspect_to_bytes(options).change_context(ProtobugError)?;
                    write_output(&bytes, output_format)
                        .change_context(ProtobugError)
                        .attach("Failed to write encoded protobuf output")?;
                }
                None => {
                    run_inspect(options).change_context(ProtobugError)?;
                }
            }
        }

        Commands::Edit {
            schema,
            message,
            file,
            filter,
            print_format,
        } => {
            let options = EditOptions {
                schema,
                message,
                file,
                filter,
            };

            match print_format {
                OutputFormatArg::Json => {
                    println!("{}", edit_to_json(options).change_context(ProtobugError)?);
                }
                output_format => {
                    let bytes = edit_to_bytes(options).change_context(ProtobugError)?;
                    write_output(&bytes, output_format)
                        .change_context(ProtobugError)
                        .attach("Failed to write encoded protobuf output")?;
                }
            }
        }
    }

    Ok(())
}
