use base64::Engine as _;
use error_stack::{Report, ResultExt as _};
use jaq_core::{
    Ctx,
    load::{Arena, File, Loader},
};
use jaq_json::Val as JaqVal;

use crate::{
    error::Inspect,
    message::{InputFormat, Inspector},
};

pub(crate) fn compact_json(inspector: &Inspector) -> std::result::Result<String, Report<Inspect>> {
    let value = serde_json::from_str::<serde_json::Value>(&inspector.canonical_json()?)
        .change_context(Inspect)?;
    serde_json::to_string(&value).change_context(Inspect)
}

pub(crate) fn join_lines(lines: Vec<String>, had_trailing_newline: bool) -> String {
    let mut output = lines.join("\n");
    if had_trailing_newline {
        output.push('\n');
    }
    output
}

pub(crate) fn encode_line_output(
    inspector: &Inspector,
    output_format: InputFormat,
) -> std::result::Result<String, Report<Inspect>> {
    let bytes = inspector.bytes()?;
    match output_format {
        InputFormat::Base64 => Ok(base64::prelude::BASE64_STANDARD.encode(bytes)),
        InputFormat::Hex => Ok(hex::encode(bytes)),
        InputFormat::Json => compact_json(inspector),
        InputFormat::Auto | InputFormat::Binary => Err(Report::new(Inspect)
            .attach("line-based editing only supports json, hex, or base64 output")),
    }
}

pub(crate) fn apply_json_filter(
    json: &str,
    filter: &str,
) -> std::result::Result<String, Report<Inspect>> {
    let input = serde_json::from_str::<serde_json::Value>(json)
        .attach("Input format: json")
        .change_context(Inspect)?;
    let program = File {
        code: filter,
        path: (),
    };
    let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));
    let arena = Arena::default();
    let modules = loader
        .load(&arena, program)
        .map_err(|errors| Report::new(Inspect).attach(format!("Invalid filter: {errors:?}")))?;
    let filter = jaq_core::Compiler::default()
        .with_funs(jaq_std::funs().chain(jaq_json::funs()))
        .compile(modules)
        .map_err(|errors| Report::new(Inspect).attach(format!("Invalid filter: {errors:?}")))?;
    let inputs = jaq_core::RcIter::new(core::iter::empty());
    let mut output = filter.run((Ctx::new([], &inputs), JaqVal::from(input)));
    let first = output
        .next()
        .transpose()
        .map_err(|error| Report::new(Inspect).attach(format!("Filter execution failed: {error}")))?
        .ok_or_else(|| Report::new(Inspect).attach("Filter produced no results"))?;

    if output.next().is_some() {
        return Err(Report::new(Inspect).attach(
            "Filter produced multiple results; use a filter that yields exactly one JSON value",
        ));
    }

    let value = serde_json::from_str::<serde_json::Value>(&first.to_string())
        .attach("Filter output was not valid JSON")
        .change_context(Inspect)?;

    serde_json::to_string_pretty(&value).change_context(Inspect)
}
