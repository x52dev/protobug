use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read as _},
};

use base64::prelude::*;
use camino::{Utf8Path, Utf8PathBuf};
use error_stack::{IntoReportCompat as _, Report, ResultExt as _};
use jaq_core::{
    Ctx,
    load::{Arena, File, Loader},
};
use jaq_json::Val as JaqVal;
use protobuf::{
    MessageDyn,
    descriptor::FileDescriptorProto,
    reflect::{
        FileDescriptor, MessageDescriptor, ReflectFieldRef, ReflectValueBox, ReflectValueRef,
        RuntimeFieldType, RuntimeType,
    },
    text_format,
};

use crate::{
    error::{Inspect, InvalidSchema, MultipleTopLevelMessages, NoTopLevelMessages},
    selection::{self, FieldPath, ProtobufLine},
    tui,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectOptions {
    pub schema: Utf8PathBuf,
    pub message: Option<String>,
    pub file: Option<Utf8PathBuf>,
    pub input_format: InputFormat,
    pub display_options: DisplayOptions,
    pub save_targets: SaveTargets,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditOptions {
    pub schema: Utf8PathBuf,
    pub message: Option<String>,
    pub file: Option<Utf8PathBuf>,
    pub filter: Option<String>,
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
        omitted_default_enum_hint(&*self.data, &self.md, selected_path)
    }

    pub(crate) fn enum_selection(
        &self,
        selected_path: &[selection::FieldPathSegment],
    ) -> Option<EnumSelection> {
        enum_selection(&*self.data, &self.md, selected_path)
    }

    pub(crate) fn cycle_enum_variant(
        &mut self,
        selected_path: &[selection::FieldPathSegment],
        delta: isize,
    ) -> Option<String> {
        let next_variant = cycle_enum_variant(&mut *self.data, &self.md, selected_path, delta)?;
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
            return Err(Report::new(Inspect)
                .attach("No save targets were configured. Pass --save-* paths to enable Ctrl-S."));
        }

        let mut saved = Vec::new();
        let bytes = self.bytes()?;

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

pub fn run_inspect(options: InspectOptions) -> std::result::Result<(), Report<Inspect>> {
    let save_targets = options.save_targets.clone();
    let display_options = options.display_options;
    let inspector = inspect(options)?;

    let mut terminal = tui::Session::new().change_context(Inspect)?;
    let mut app =
        tui::App::new(inspector, save_targets, display_options).change_context(Inspect)?;
    app.run(terminal.terminal_mut()).change_context(Inspect)?;

    Ok(())
}

pub fn inspect_to_json(options: InspectOptions) -> std::result::Result<String, Report<Inspect>> {
    inspect(options)?.canonical_json()
}

pub fn inspect_to_bytes(options: InspectOptions) -> std::result::Result<Vec<u8>, Report<Inspect>> {
    inspect(options)?.bytes()
}

pub fn edit_to_json(options: EditOptions) -> std::result::Result<String, Report<Inspect>> {
    edit(options)?.canonical_json()
}

pub fn edit_to_bytes(options: EditOptions) -> std::result::Result<Vec<u8>, Report<Inspect>> {
    edit(options)?.bytes()
}

fn inspect(options: InspectOptions) -> std::result::Result<Inspector, Report<Inspect>> {
    let input = read_input(options.file.as_deref())?;
    load_inspector(
        options.schema.as_ref(),
        options.message.as_deref(),
        &input,
        options.input_format,
    )
}

fn edit(options: EditOptions) -> std::result::Result<Inspector, Report<Inspect>> {
    let input = read_input(options.file.as_deref())?;
    let json = json_input_as_text(&input)?;
    let filtered = match options.filter.as_deref() {
        Some(filter) => apply_json_filter(json, filter)?,
        None => json.to_owned(),
    };

    load_inspector(
        options.schema.as_ref(),
        options.message.as_deref(),
        filtered.as_bytes(),
        InputFormat::Json,
    )
}

pub fn validate_schema(
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

    let tf = text_format::print_to_string_pretty(fd);
    Ok(tf)
}

pub fn load_inspector(
    schema: &Utf8Path,
    message: Option<&str>,
    raw_input: &[u8],
    input_format: InputFormat,
) -> std::result::Result<Inspector, Report<Inspect>> {
    let md = load_message_descriptor(schema, message)?;
    let msg = match input_format {
        InputFormat::Json => {
            protobuf_json_mapping::parse_dyn_from_str(&md, json_input_as_text(raw_input)?)
                .attach_with(|| format!("Message type: {}", md.name_to_package()))
                .change_context(Inspect)?
        }
        _ => {
            let decoded = decode_input(raw_input, input_format)?;
            md.parse_from_bytes(&decoded)
                .attach_with(|| format!("Message type: {}", md.name_to_package()))
                .change_context(Inspect)?
        }
    };

    Ok(Inspector::new(md, msg))
}

pub fn available_message_names(fd: &FileDescriptor) -> Vec<String> {
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

fn read_input(path: Option<&Utf8Path>) -> std::result::Result<Vec<u8>, Report<Inspect>> {
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

fn json_input_as_text(raw_input: &[u8]) -> std::result::Result<&str, Report<Inspect>> {
    std::str::from_utf8(raw_input)
        .attach("Input format: json")
        .change_context(Inspect)
}

fn apply_json_filter(json: &str, filter: &str) -> std::result::Result<String, Report<Inspect>> {
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

fn load_message_descriptor(
    schema: &Utf8Path,
    message: Option<&str>,
) -> std::result::Result<MessageDescriptor, Report<Inspect>> {
    let fd = load_file_descriptor(schema)?;
    select_message(&fd, message)
        .attach_with(|| format!("Schema file: {schema}"))
        .change_context(Inspect)
}

fn load_file_descriptor(schema: &Utf8Path) -> std::result::Result<FileDescriptor, Report<Inspect>> {
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

fn select_message(
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

fn decode_input(
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

fn decode_base64(raw_input: &[u8]) -> std::result::Result<Vec<u8>, Report<Inspect>> {
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

fn decode_hex(raw_input: &[u8]) -> std::result::Result<Vec<u8>, Report<Inspect>> {
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

fn omitted_default_enum_hint(
    message: &dyn MessageDyn,
    descriptor: &MessageDescriptor,
    path: &[selection::FieldPathSegment],
) -> Option<String> {
    let (selection::FieldPathSegment::Field(field_name), rest) = path.split_first()? else {
        return None;
    };

    let field = descriptor.field_by_name(field_name)?;

    if rest.is_empty() {
        let RuntimeFieldType::Singular(RuntimeType::Enum(enum_descriptor)) =
            field.runtime_field_type()
        else {
            return None;
        };

        if field.has_field(message) {
            return None;
        }

        let ReflectValueRef::Enum(_, value_number) = field.get_singular_field_or_default(message)
        else {
            return None;
        };
        let ReflectValueRef::Enum(_, default_number) = field.singular_default_value() else {
            return None;
        };

        if value_number != default_number {
            return None;
        }

        let variant = enum_descriptor.value_by_number(value_number)?;

        return Some(format!(
            "Default enum {} is omitted on the wire",
            variant.name(),
        ));
    }

    match field.get_reflect(message) {
        ReflectFieldRef::Optional(optional) => {
            let ReflectValueRef::Message(nested) = optional.value()? else {
                return None;
            };

            omitted_default_enum_hint(&*nested, &nested.descriptor_dyn(), rest)
        }
        ReflectFieldRef::Repeated(repeated) => {
            let (selection::FieldPathSegment::Index(index), nested_path) = rest.split_first()?
            else {
                return None;
            };
            let nested = repeated.get(*index);

            if nested_path.is_empty() {
                return None;
            }

            let ReflectValueRef::Message(nested) = nested else {
                return None;
            };

            omitted_default_enum_hint(&*nested, &nested.descriptor_dyn(), nested_path)
        }
        ReflectFieldRef::Map(_) => None,
    }
}

fn enum_selection(
    message: &dyn MessageDyn,
    descriptor: &MessageDescriptor,
    path: &[selection::FieldPathSegment],
) -> Option<EnumSelection> {
    let (selection::FieldPathSegment::Field(field_name), rest) = path.split_first()? else {
        return None;
    };

    let field = descriptor.field_by_name(field_name)?;

    match (field.runtime_field_type(), rest) {
        (RuntimeFieldType::Singular(RuntimeType::Enum(enum_descriptor)), []) => {
            let ReflectValueRef::Enum(_, current_number) =
                field.get_singular_field_or_default(message)
            else {
                return None;
            };
            enum_selection_for_number(&enum_descriptor, current_number)
        }
        (
            RuntimeFieldType::Repeated(RuntimeType::Enum(enum_descriptor)),
            [selection::FieldPathSegment::Index(index)],
        ) => {
            let repeated = field.get_repeated(message);
            let ReflectValueRef::Enum(_, current_number) = repeated.get(*index) else {
                return None;
            };
            enum_selection_for_number(&enum_descriptor, current_number)
        }
        (_, _) => match field.get_reflect(message) {
            ReflectFieldRef::Optional(optional) => {
                let ReflectValueRef::Message(nested) = optional.value()? else {
                    return None;
                };
                enum_selection(&*nested, &nested.descriptor_dyn(), rest)
            }
            ReflectFieldRef::Repeated(repeated) => {
                let (selection::FieldPathSegment::Index(index), nested_path) =
                    rest.split_first()?
                else {
                    return None;
                };
                let ReflectValueRef::Message(nested) = repeated.get(*index) else {
                    return None;
                };
                enum_selection(&*nested, &nested.descriptor_dyn(), nested_path)
            }
            ReflectFieldRef::Map(_) => None,
        },
    }
}

fn enum_selection_for_number(
    enum_descriptor: &protobuf::reflect::EnumDescriptor,
    current_number: i32,
) -> Option<EnumSelection> {
    let variants = enum_descriptor.values().collect::<Vec<_>>();
    let current = variants
        .iter()
        .position(|variant| variant.value() == current_number)
        .unwrap_or_default();

    Some(EnumSelection {
        variants: variants
            .into_iter()
            .map(|variant| variant.name().to_owned())
            .collect(),
        current,
    })
}

fn cycle_enum_variant(
    message: &mut dyn MessageDyn,
    descriptor: &MessageDescriptor,
    path: &[selection::FieldPathSegment],
    delta: isize,
) -> Option<String> {
    let (selection::FieldPathSegment::Field(field_name), rest) = path.split_first()? else {
        return None;
    };

    let field = descriptor.field_by_name(field_name)?;

    match (field.runtime_field_type(), rest) {
        (RuntimeFieldType::Singular(RuntimeType::Enum(enum_descriptor)), []) => {
            let ReflectValueRef::Enum(_, current_number) =
                field.get_singular_field_or_default(message)
            else {
                return None;
            };
            let next_variant = cycle_enum_descriptor(&enum_descriptor, current_number, delta)?;
            field.set_singular_field(message, ReflectValueBox::from(next_variant.clone()));
            Some(next_variant.name().to_owned())
        }
        (
            RuntimeFieldType::Repeated(RuntimeType::Enum(enum_descriptor)),
            [selection::FieldPathSegment::Index(index)],
        ) => {
            let repeated = field.mut_repeated(message);
            let ReflectValueRef::Enum(_, current_number) = repeated.get(*index) else {
                return None;
            };
            let next_variant = cycle_enum_descriptor(&enum_descriptor, current_number, delta)?;
            let mut repeated = field.mut_repeated(message);
            repeated.set(*index, ReflectValueBox::from(next_variant.clone()));
            Some(next_variant.name().to_owned())
        }
        (_, _) => match field.runtime_field_type() {
            RuntimeFieldType::Singular(RuntimeType::Message(message_descriptor)) => {
                let nested = field.mut_message(message);
                cycle_enum_variant(nested, &message_descriptor, rest, delta)
            }
            RuntimeFieldType::Repeated(RuntimeType::Message(message_descriptor)) => {
                let (selection::FieldPathSegment::Index(index), nested_path) =
                    rest.split_first()?
                else {
                    return None;
                };

                let mut repeated = field.mut_repeated(message);
                let mut nested = repeated.get(*index).to_box();
                let ReflectValueBox::Message(nested_message) = &mut nested else {
                    return None;
                };
                let next_variant = cycle_enum_variant(
                    &mut **nested_message,
                    &message_descriptor,
                    nested_path,
                    delta,
                )?;
                repeated.set(*index, nested);
                Some(next_variant)
            }
            _ => None,
        },
    }
}

fn cycle_enum_descriptor(
    enum_descriptor: &protobuf::reflect::EnumDescriptor,
    current_number: i32,
    delta: isize,
) -> Option<protobuf::reflect::EnumValueDescriptor> {
    let variants = enum_descriptor.values().collect::<Vec<_>>();
    let current = variants
        .iter()
        .position(|variant| variant.value() == current_number)
        .unwrap_or_default();
    let next = wrap_index(current, variants.len(), delta)?;
    Some(variants[next].clone())
}

fn wrap_index(current: usize, len: usize, delta: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }

    let current = current as isize;
    let len = len as isize;

    Some((current + delta).rem_euclid(len) as usize)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use indoc::indoc;
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use protobuf::{
        EnumOrUnknown, Message as _, MessageField, SpecialFields,
        well_known_types::timestamp::Timestamp,
    };
    use protogen::system_event::{
        SystemEvent,
        system_event::{Event as SystemEventVariant, MouseButton, MouseDown},
    };
    use tempfile::tempdir;

    use super::*;

    fn schema_path() -> Utf8PathBuf {
        Utf8PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../protogen/proto/system-event.proto"
        ))
    }

    fn sample_bytes() -> Vec<u8> {
        SystemEvent {
            timestamp: MessageField::some(Timestamp {
                seconds: 1_234_567,
                nanos: 123,
                special_fields: SpecialFields::default(),
            }),
            reason: Some("user clicked".to_owned()),
            event: Some(SystemEventVariant::Click(MouseDown {
                button: EnumOrUnknown::new(MouseButton::Left),
                x: 42,
                y: 100,
                ..Default::default()
            })),
            special_fields: SpecialFields::default(),
        }
        .write_to_bytes()
        .unwrap()
    }

    fn sample_json() -> String {
        load_inspector(
            schema_path().as_ref(),
            Some("SystemEvent"),
            &sample_bytes(),
            InputFormat::Binary,
        )
        .unwrap()
        .canonical_json()
        .unwrap()
    }

    #[test]
    fn auto_detects_base64_and_hex_input() {
        let bytes = sample_bytes();
        let base64 = BASE64_STANDARD.encode(&bytes);
        let hex = hex::encode(&bytes);

        assert_eq!(
            decode_input(base64.as_bytes(), InputFormat::Auto).unwrap(),
            bytes,
        );
        assert_eq!(
            decode_input(hex.as_bytes(), InputFormat::Auto).unwrap(),
            bytes
        );
    }

    #[test]
    fn read_input_requires_explicit_stdin_marker() {
        let error = read_input(None).unwrap_err();
        let message = format!("{error:?}");

        assert!(message.contains("No input file was provided."));
        assert!(message.contains("Pass --file <path> or --file - to read from stdin."));
    }

    #[test]
    fn explicit_hex_format_ignores_whitespace() {
        let bytes = sample_bytes();
        let spaced_hex = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(" ");

        assert_eq!(
            decode_input(spaced_hex.as_bytes(), InputFormat::Hex).unwrap(),
            bytes,
        );
    }

    #[test]
    fn selecting_message_is_required_when_schema_has_multiple_messages() {
        let dir = tempdir().unwrap();
        let schema_path = Utf8PathBuf::from_path_buf(dir.path().join("multi.proto")).unwrap();
        fs::write(
            &schema_path,
            indoc! {r#"
                syntax = "proto3";

                message Alpha {}
                message Beta {}
            "#},
        )
        .unwrap();

        let fd = load_file_descriptor(schema_path.as_ref()).unwrap();
        assert_eq!(available_message_names(&fd), vec!["Alpha", "Beta"]);
        assert!(select_message(&fd, None).is_err());
        assert_eq!(select_message(&fd, Some("Beta")).unwrap().name(), "Beta");
    }

    #[test]
    fn inspector_tracks_parse_errors_without_losing_last_valid_message() {
        let mut inspector = load_inspector(
            schema_path().as_ref(),
            Some("SystemEvent"),
            &sample_bytes(),
            InputFormat::Binary,
        )
        .unwrap();

        let original = inspector.bytes().unwrap();

        assert!(inspector.apply_json("{ not valid json").is_err());
        assert!(inspector.parse_error().is_some());
        assert_eq!(inspector.bytes().unwrap(), original);

        let mut value =
            serde_json::from_str::<serde_json::Value>(&inspector.canonical_json().unwrap())
                .unwrap();
        value["reason"] = serde_json::Value::String("updated".to_owned());

        inspector
            .apply_json(&serde_json::to_string_pretty(&value).unwrap())
            .unwrap();

        assert_eq!(inspector.parse_error(), None);
        assert!(inspector.canonical_json().unwrap().contains("\"updated\""));
    }

    #[test]
    fn canonical_json_matches_snapshot() {
        let inspector = load_inspector(
            schema_path().as_ref(),
            Some("SystemEvent"),
            &sample_bytes(),
            InputFormat::Binary,
        )
        .unwrap();

        assert_snapshot!(inspector.canonical_json().unwrap());
    }

    #[test]
    fn json_filter_matches_snapshot() {
        let filtered = apply_json_filter(
            &sample_json(),
            r#".reason = "patched with jaq" | .click.button = "Right" | .click.x = 7 | .click.y = 9 | .timestamp.nanos = 456 | .timestamp.seconds = "1234568""#,
        )
        .unwrap();

        assert_snapshot!(filtered);
    }

    #[test]
    fn json_input_round_trips_sample_bytes() {
        let bytes = load_inspector(
            schema_path().as_ref(),
            Some("SystemEvent"),
            sample_json().as_bytes(),
            InputFormat::Json,
        )
        .unwrap()
        .bytes()
        .unwrap();

        assert_eq!(bytes, sample_bytes());
    }

    #[test]
    fn filtered_edit_bytes_match_snapshot() {
        let dir = tempdir().unwrap();
        let input_path = Utf8PathBuf::from_path_buf(dir.path().join("input.json")).unwrap();
        fs::write(&input_path, sample_json()).unwrap();
        let bytes = edit_to_bytes(EditOptions {
            schema: schema_path(),
            message: Some("SystemEvent".to_owned()),
            file: Some(input_path),
            filter: Some(
                r#".reason = "patched with jaq" | .click.button = "Right" | .click.x = 7 | .click.y = 9 | .timestamp.nanos = 456 | .timestamp.seconds = "1234568""#
                    .to_owned(),
            ),
        })
        .unwrap();

        assert_snapshot!(hex::encode(&bytes));
    }

    #[test]
    fn inspector_saves_all_configured_output_formats() {
        let dir = tempdir().unwrap();
        let bytes = sample_bytes();
        let inspector = load_inspector(
            schema_path().as_ref(),
            Some("SystemEvent"),
            &bytes,
            InputFormat::Binary,
        )
        .unwrap();

        let targets = SaveTargets {
            json: Some(Utf8PathBuf::from_path_buf(dir.path().join("message.json")).unwrap()),
            base64: Some(Utf8PathBuf::from_path_buf(dir.path().join("message.base64")).unwrap()),
            hex: Some(Utf8PathBuf::from_path_buf(dir.path().join("message.hex")).unwrap()),
            binary: Some(Utf8PathBuf::from_path_buf(dir.path().join("message.bin")).unwrap()),
        };

        let saved = inspector.save(&targets).unwrap();

        assert_eq!(saved.len(), 4);
        assert_eq!(
            fs::read_to_string(targets.base64.as_ref().unwrap()).unwrap(),
            BASE64_STANDARD.encode(&bytes),
        );
        assert_eq!(
            fs::read_to_string(targets.hex.as_ref().unwrap()).unwrap(),
            hex::encode(&bytes),
        );
        assert_eq!(fs::read(targets.binary.as_ref().unwrap()).unwrap(), bytes);
        assert!(
            fs::read_to_string(targets.json.as_ref().unwrap())
                .unwrap()
                .contains("\"user clicked\"")
        );
    }

    #[test]
    fn inspector_reports_omitted_default_enum_hint() {
        let inspector = load_inspector(
            schema_path().as_ref(),
            Some("SystemEvent"),
            &sample_bytes(),
            InputFormat::Binary,
        )
        .unwrap();

        let hint = inspector.omitted_default_enum_hint(&[
            selection::FieldPathSegment::Field("click".to_owned()),
            selection::FieldPathSegment::Field("button".to_owned()),
        ]);

        assert_eq!(
            hint.as_deref(),
            Some("Default enum Left is omitted on the wire"),
        );
    }

    #[test]
    fn inspector_lists_and_cycles_enum_variants() {
        let mut inspector = load_inspector(
            schema_path().as_ref(),
            Some("SystemEvent"),
            &sample_bytes(),
            InputFormat::Binary,
        )
        .unwrap();

        assert_eq!(
            inspector.enum_selection(&[
                selection::FieldPathSegment::Field("click".to_owned()),
                selection::FieldPathSegment::Field("button".to_owned()),
            ]),
            Some(EnumSelection {
                variants: vec!["Left".to_owned(), "Right".to_owned(), "Middle".to_owned()],
                current: 0,
            }),
        );

        assert_eq!(
            inspector.cycle_enum_variant(
                &[
                    selection::FieldPathSegment::Field("click".to_owned()),
                    selection::FieldPathSegment::Field("button".to_owned()),
                ],
                1,
            ),
            Some("Right".to_owned()),
        );
        assert!(
            inspector
                .canonical_json()
                .unwrap()
                .contains(r#""button": "Right""#)
        );
    }

    #[test]
    fn inspector_skips_omitted_default_enum_hint_for_other_fields() {
        let inspector = load_inspector(
            schema_path().as_ref(),
            Some("SystemEvent"),
            &sample_bytes(),
            InputFormat::Binary,
        )
        .unwrap();

        assert_eq!(
            inspector.omitted_default_enum_hint(&[
                selection::FieldPathSegment::Field("click".to_owned()),
                selection::FieldPathSegment::Field("x".to_owned()),
            ]),
            None,
        );
    }

    #[test]
    fn validate_schema_returns_the_requested_schema_descriptor() {
        let descriptor = validate_schema(schema_path()).unwrap();

        assert!(descriptor.contains("name: \"SystemEvent\""));
        assert!(!descriptor.contains("name: \"Timestamp\""));
    }
}
