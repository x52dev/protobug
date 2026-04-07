//! Protobuf Debugging Suite.

use std::io::Write as _;

use base64::Engine as _;
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand, ValueEnum};
use derive_more::derive::{Display, Error};
use error_stack::{Report, ResultExt as _};
use protobug::{
    DisplayOptions, EditOptions, InputFormat, InspectOptions, SaveTargets, edit_in_place,
    edit_to_bytes, edit_to_encoded_lines, edit_to_json, edit_to_json_lines, inspect_to_bytes,
    inspect_to_json, run_inspect, validate_schema,
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

        /// Treat the input as one hex/base64 payload per line and inspect one message at a time.
        #[arg(long)]
        multiple: bool,

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

    /// Edits protobuf payloads by applying a jaq filter to their JSON representation.
    Edit {
        #[arg(long)]
        schema: Utf8PathBuf,

        /// Message name relative to the package in the schema.
        #[arg(long)]
        message: Option<String>,

        /// Input file path. Pass "-" to read from stdin.
        #[arg(long)]
        file: Option<Utf8PathBuf>,

        /// How to decode the input payload before applying the filter.
        #[arg(long, value_enum, default_value_t = EditInputFormatArg::Auto)]
        input_format: EditInputFormatArg,

        /// Jaq filter to run against the input JSON before protobuf encoding.
        #[arg(long)]
        filter: Option<String>,

        /// Treat the input as one hex/base64 payload per line and edit each line independently.
        #[arg(long)]
        multiple: bool,

        /// Overwrite the input file using the input file's encoding.
        #[arg(long, conflicts_with = "print_format")]
        in_place: bool,

        /// Print the edited message in the selected format.
        #[arg(long, value_enum)]
        print_format: Option<OutputFormatArg>,
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

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EditInputFormatArg {
    Auto,
    Json,
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

impl From<EditInputFormatArg> for InputFormat {
    fn from(value: EditInputFormatArg) -> Self {
        match value {
            EditInputFormatArg::Auto => Self::Auto,
            EditInputFormatArg::Json => Self::Json,
            EditInputFormatArg::Base64 => Self::Base64,
            EditInputFormatArg::Hex => Self::Hex,
            EditInputFormatArg::Binary => Self::Binary,
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

fn default_edit_output_format(input_format: EditInputFormatArg, multiple: bool) -> OutputFormatArg {
    if multiple {
        match input_format {
            EditInputFormatArg::Base64 => OutputFormatArg::Base64,
            EditInputFormatArg::Hex => OutputFormatArg::Hex,
            _ => OutputFormatArg::Binary,
        }
    } else {
        OutputFormatArg::Binary
    }
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
            multiple,
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
                multiple,
                display_options: DisplayOptions {
                    columns,
                    ..Default::default()
                },
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
            input_format,
            filter,
            multiple,
            in_place,
            print_format,
        } => {
            let options = EditOptions {
                schema,
                message,
                file,
                input_format: input_format.into(),
                filter,
                multiple,
            };

            if in_place {
                edit_in_place(options).change_context(ProtobugError)?;
            } else {
                match print_format
                    .unwrap_or_else(|| default_edit_output_format(input_format, multiple))
                {
                    OutputFormatArg::Json if multiple => {
                        print!(
                            "{}",
                            edit_to_json_lines(options).change_context(ProtobugError)?
                        );
                    }
                    OutputFormatArg::Json => {
                        println!("{}", edit_to_json(options).change_context(ProtobugError)?);
                    }
                    OutputFormatArg::Hex if multiple => {
                        print!(
                            "{}",
                            edit_to_encoded_lines(options, InputFormat::Hex)
                                .change_context(ProtobugError)?
                        );
                    }
                    OutputFormatArg::Base64 if multiple => {
                        print!(
                            "{}",
                            edit_to_encoded_lines(options, InputFormat::Base64)
                                .change_context(ProtobugError)?
                        );
                    }
                    OutputFormatArg::Binary if multiple => {
                        return Err(Report::new(ProtobugError).attach(
                            "`edit --multiple` does not support binary stdout; use `--print-format hex`, `base64`, or `json`",
                        ));
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
    }

    Ok(())
}
