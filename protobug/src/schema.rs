use camino::{Utf8Path, Utf8PathBuf};
use error_stack::{IntoReportCompat as _, Report, ResultExt as _};
use protobuf::{
    descriptor::FileDescriptorProto,
    reflect::{FileDescriptor, MessageDescriptor},
    text_format,
};

use crate::{
    decode,
    error::{Inspect, InvalidSchema, MultipleTopLevelMessages, NoTopLevelMessages},
    message::{InputFormat, Inspector},
};

pub(crate) fn validate_schema(
    schema_path: Utf8PathBuf,
) -> std::result::Result<String, Report<anyhow::Error>> {
    let fds = protobuf_parse::Parser::new()
        .pure()
        .includes(schema_path.parent().as_slice())
        .input(&schema_path)
        .parse_and_typecheck()
        .into_report()?;

    let schema_name = schema_path.file_name().unwrap_or(schema_path.as_str());
    let fd = fds
        .file_descriptors
        .iter()
        .find(|fd| fd.name() == schema_name)
        .or_else(|| fds.file_descriptors.last())
        .ok_or_else(|| anyhow::anyhow!("No file descriptors resolved from schema: {schema_path}"))
        .into_report()?;

    Ok(text_format::print_to_string_pretty(fd))
}

pub(crate) fn load_inspector(
    schema: &Utf8Path,
    message: Option<&str>,
    raw_input: &[u8],
    input_format: InputFormat,
) -> std::result::Result<Inspector, Report<Inspect>> {
    let md = load_message_descriptor(schema, message)?;
    let msg = match input_format {
        InputFormat::Json => {
            protobuf_json_mapping::parse_dyn_from_str(&md, decode::json_input_as_text(raw_input)?)
                .attach_with(|| format!("Message type: {}", md.name_to_package()))
                .change_context(Inspect)?
        }
        _ => {
            let decoded = decode::decode_input(raw_input, input_format)?;
            md.parse_from_bytes(&decoded)
                .attach_with(|| format!("Message type: {}", md.name_to_package()))
                .change_context(Inspect)?
        }
    };

    Ok(Inspector::new(md, msg))
}

pub(crate) fn available_message_names(fd: &FileDescriptor) -> Vec<String> {
    let mut names = Vec::new();

    for message in fd.messages() {
        collect_message_names(message, &mut names);
    }

    names.sort();
    names
}

fn collect_message_names(message: MessageDescriptor, names: &mut Vec<String>) {
    names.push(message.name_to_package().to_owned());

    for nested in message.nested_messages() {
        collect_message_names(nested, names);
    }
}

pub(crate) fn load_file_descriptor(
    schema: &Utf8Path,
) -> std::result::Result<FileDescriptor, Report<Inspect>> {
    let fds = protobuf_parse::Parser::new()
        .pure()
        .includes(schema.parent().as_slice())
        .input(schema)
        .parse_and_typecheck()
        .into_report()
        .attach_with(|| format!("Schema file: {schema}"))
        .change_context(Inspect)?;

    let descriptors = build_file_descriptors(fds.file_descriptors)?;
    let schema_name = schema.file_name().unwrap_or(schema.as_str());

    descriptors
        .iter()
        .find(|fd| fd.name() == schema_name)
        .cloned()
        .or_else(|| descriptors.last().cloned())
        .ok_or_else(|| {
            Report::new(Inspect).attach(format!(
                "No file descriptors resolved from schema: {schema}"
            ))
        })
}

fn build_file_descriptors(
    protos: Vec<FileDescriptorProto>,
) -> std::result::Result<Vec<FileDescriptor>, Report<Inspect>> {
    let mut pending = protos;
    let mut built: Vec<FileDescriptor> = Vec::new();

    while !pending.is_empty() {
        let before = pending.len();
        let mut index = 0;

        while index < pending.len() {
            let proto = &pending[index];
            let ready = proto
                .dependency
                .iter()
                .all(|dep| built.iter().any(|fd| fd.name() == dep.as_str()));

            if ready {
                let proto = pending.remove(index);
                let deps = proto
                    .dependency
                    .iter()
                    .map(|dep| {
                        built
                            .iter()
                            .find(|fd| fd.name() == dep.as_str())
                            .cloned()
                            .expect("ready dependencies should always be present")
                    })
                    .collect::<Vec<_>>();

                let fd = FileDescriptor::new_dynamic(proto, &deps).change_context(Inspect)?;
                built.push(fd);
            } else {
                index += 1;
            }
        }

        if pending.len() == before {
            let unresolved = pending
                .iter()
                .map(|proto| proto.name().to_owned())
                .collect::<Vec<_>>();

            return Err(Report::new(Inspect).attach(format!(
                "Could not resolve descriptor dependencies for: {}",
                unresolved.join(", "),
            )));
        }
    }

    Ok(built)
}

pub(crate) fn select_message(
    fd: &FileDescriptor,
    message: Option<&str>,
) -> std::result::Result<MessageDescriptor, Report<InvalidSchema>> {
    let names = available_message_names(fd);

    match message {
        Some(name) => fd.message_by_package_relative_name(name).ok_or_else(|| {
            Report::new(InvalidSchema)
                .attach(format!("Requested message: {name}"))
                .attach(format!("Available messages: {}", names.join(", ")))
        }),
        None if names.is_empty() => Err(NoTopLevelMessages).change_context(InvalidSchema),
        None if names.len() == 1 => {
            let name = &names[0];
            fd.message_by_package_relative_name(name).ok_or_else(|| {
                Report::new(InvalidSchema).attach(format!("Resolved message disappeared: {name}"))
            })
        }
        None => Err(MultipleTopLevelMessages)
            .attach(format!("Available messages: {}", names.join(", ")))
            .change_context(InvalidSchema),
    }
}

fn load_message_descriptor(
    schema: &Utf8Path,
    message: Option<&str>,
) -> std::result::Result<MessageDescriptor, Report<Inspect>> {
    let fd = load_file_descriptor(schema)?;
    select_message(&fd, message)
        .attach_with(|| format!("Schema file: {schema}"))
        .change_context(Inspect)
}
