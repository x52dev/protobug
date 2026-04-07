#[cfg(test)]
mod tests;

use camino::Utf8PathBuf;
use error_stack::{Report, ResultExt as _};

use crate::{
    decode,
    error::Inspect,
    message::{DisplayOptions, InputFormat, Inspector, SaveTargets},
    schema::load_inspector,
    tui,
};

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

pub(crate) fn inspect_multiple(
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
