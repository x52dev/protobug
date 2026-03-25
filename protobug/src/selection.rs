use std::{
    collections::{BTreeSet, HashMap},
    ops::Range,
};

use protobuf::{
    MessageDyn,
    reflect::{
        FieldDescriptor, MessageDescriptor, ReflectFieldRef, ReflectValueRef, RuntimeFieldType,
        RuntimeType,
    },
};

pub(crate) type FieldPath = Vec<FieldPathSegment>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum FieldPathSegment {
    Field(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProtobufLine {
    pub(crate) path: FieldPath,
    pub(crate) text: String,
}

pub(crate) fn selected_path_for_json_cursor(
    descriptor: &MessageDescriptor,
    json: &str,
    cursor: (usize, usize),
) -> Option<FieldPath> {
    let offset = byte_offset_for_cursor(json, cursor)?;
    let root = JsonParser::new(json).parse().ok()?;
    let raw_path = root.path_at(offset)?;
    normalize_path(descriptor, &raw_path)
}

pub(crate) fn protobuf_lines(
    descriptor: &MessageDescriptor,
    message: &dyn MessageDyn,
) -> Vec<ProtobufLine> {
    let mut lines = Vec::new();
    render_message(descriptor, message, &mut Vec::new(), 0, &mut lines);
    lines
}

pub(crate) fn highlighted_byte_indices(
    descriptor: &MessageDescriptor,
    bytes: &[u8],
    selected_path: &[FieldPathSegment],
) -> BTreeSet<usize> {
    let mut occurrences = Vec::new();
    collect_occurrences(descriptor, bytes, 0, &mut Vec::new(), &mut occurrences);

    let mut highlighted = BTreeSet::new();

    for occurrence in occurrences {
        if path_is_prefix(selected_path, &occurrence.path) {
            highlighted.extend(occurrence.range);
        }
    }

    highlighted
}

pub(crate) fn related_path(selected: &[FieldPathSegment], candidate: &[FieldPathSegment]) -> bool {
    path_is_prefix(selected, candidate) || path_is_prefix(candidate, selected)
}

fn path_is_prefix(prefix: &[FieldPathSegment], path: &[FieldPathSegment]) -> bool {
    prefix.len() <= path.len() && prefix.iter().zip(path).all(|(left, right)| left == right)
}

fn normalize_path(
    descriptor: &MessageDescriptor,
    raw_path: &[FieldPathSegment],
) -> Option<FieldPath> {
    let mut normalized = Vec::new();
    let mut current = descriptor.clone();
    let mut current_field: Option<FieldDescriptor> = None;

    for segment in raw_path {
        match segment {
            FieldPathSegment::Field(name) => {
                let field = match current.field_by_name_or_json_name(name) {
                    Some(field) => field,
                    None if normalized.is_empty() => return None,
                    None => break,
                };

                normalized.push(FieldPathSegment::Field(field.name().to_owned()));

                current_field = Some(field.clone());

                match field.runtime_field_type() {
                    RuntimeFieldType::Singular(RuntimeType::Message(next))
                    | RuntimeFieldType::Repeated(RuntimeType::Message(next)) => {
                        current = next;
                    }
                    RuntimeFieldType::Map(_, RuntimeType::Message(next)) => {
                        current = next;
                    }
                    _ => {}
                }
            }
            FieldPathSegment::Index(index) => {
                let Some(field) = &current_field else {
                    break;
                };

                if field.is_repeated() {
                    normalized.push(FieldPathSegment::Index(*index));
                } else {
                    break;
                }
            }
        }
    }

    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn render_message(
    descriptor: &MessageDescriptor,
    message: &dyn MessageDyn,
    parent_path: &mut FieldPath,
    indent: usize,
    lines: &mut Vec<ProtobufLine>,
) {
    for field in descriptor.fields() {
        match field.get_reflect(message) {
            ReflectFieldRef::Optional(optional) => {
                let Some(value) = optional.value() else {
                    continue;
                };

                let mut field_path = parent_path.clone();
                field_path.push(FieldPathSegment::Field(field.name().to_owned()));
                render_value(field.name(), value, &field_path, indent, lines);
            }
            ReflectFieldRef::Repeated(repeated) => {
                for index in 0..repeated.len() {
                    let mut field_path = parent_path.clone();
                    field_path.push(FieldPathSegment::Field(field.name().to_owned()));
                    field_path.push(FieldPathSegment::Index(index));
                    render_value(
                        field.name(),
                        repeated.get(index),
                        &field_path,
                        indent,
                        lines,
                    );
                }
            }
            ReflectFieldRef::Map(map) => {
                for (key, value) in &map {
                    let field_path = parent_path
                        .iter()
                        .cloned()
                        .chain([FieldPathSegment::Field(field.name().to_owned())])
                        .collect::<Vec<_>>();

                    lines.push(ProtobufLine {
                        path: field_path.clone(),
                        text: format!("{}{} {{", " ".repeat(indent), field.name()),
                    });
                    lines.push(ProtobufLine {
                        path: field_path.clone(),
                        text: format!("{}key: {}", " ".repeat(indent + 2), format_value(key)),
                    });

                    match value {
                        ReflectValueRef::Message(message) => {
                            lines.push(ProtobufLine {
                                path: field_path.clone(),
                                text: format!("{}value {{", " ".repeat(indent + 2)),
                            });
                            render_message(
                                &message.descriptor_dyn(),
                                &*message,
                                &mut field_path.clone(),
                                indent + 4,
                                lines,
                            );
                            lines.push(ProtobufLine {
                                path: field_path.clone(),
                                text: format!("{}}}", " ".repeat(indent + 2)),
                            });
                        }
                        _ => lines.push(ProtobufLine {
                            path: field_path.clone(),
                            text: format!(
                                "{}value: {}",
                                " ".repeat(indent + 2),
                                format_value(value)
                            ),
                        }),
                    }

                    lines.push(ProtobufLine {
                        path: field_path,
                        text: format!("{}}}", " ".repeat(indent)),
                    });
                }
            }
        }
    }
}

fn render_value(
    field_name: &str,
    value: ReflectValueRef<'_>,
    path: &[FieldPathSegment],
    indent: usize,
    lines: &mut Vec<ProtobufLine>,
) {
    match value {
        ReflectValueRef::Message(message) => {
            lines.push(ProtobufLine {
                path: path.to_vec(),
                text: format!("{}{} {{", " ".repeat(indent), field_name),
            });
            render_message(
                &message.descriptor_dyn(),
                &*message,
                &mut path.to_vec(),
                indent + 2,
                lines,
            );
            lines.push(ProtobufLine {
                path: path.to_vec(),
                text: format!("{}}}", " ".repeat(indent)),
            });
        }
        _ => lines.push(ProtobufLine {
            path: path.to_vec(),
            text: format!(
                "{}{}: {}",
                " ".repeat(indent),
                field_name,
                format_value(value)
            ),
        }),
    }
}

fn format_value(value: ReflectValueRef<'_>) -> String {
    match value {
        ReflectValueRef::String(value) => format!("{value:?}"),
        ReflectValueRef::Bytes(value) => format!("{:?}", value),
        _ => value.to_string(),
    }
}

#[derive(Debug, Clone)]
struct FieldOccurrence {
    path: FieldPath,
    range: Range<usize>,
}

fn collect_occurrences(
    descriptor: &MessageDescriptor,
    bytes: &[u8],
    base_offset: usize,
    parent_path: &mut FieldPath,
    occurrences: &mut Vec<FieldOccurrence>,
) {
    let mut offset = 0;
    let mut repeated_indices = HashMap::<u32, usize>::new();

    while offset < bytes.len() {
        let field_start = offset;
        let Some(tag) = read_varint(bytes, &mut offset) else {
            break;
        };
        let field_number = (tag >> 3) as u32;
        let wire_type = (tag & 0x07) as u8;
        let Some(field) = descriptor.field_by_number(field_number) else {
            if skip_unknown_field(bytes, &mut offset, wire_type).is_none() {
                break;
            }
            continue;
        };

        let mut field_path = parent_path.clone();
        field_path.push(FieldPathSegment::Field(field.name().to_owned()));

        let occurrence_path = if field.is_repeated() {
            let index = repeated_indices.entry(field_number).or_insert(0);
            let mut repeated_path = field_path.clone();
            repeated_path.push(FieldPathSegment::Index(*index));
            *index += 1;
            repeated_path
        } else {
            field_path.clone()
        };

        match parse_field(bytes, &mut offset, wire_type) {
            Some(ParsedField::Value(range)) => {
                occurrences.push(FieldOccurrence {
                    path: occurrence_path,
                    range: (base_offset + field_start)..(base_offset + range.end),
                });
            }
            Some(ParsedField::LengthDelimited {
                full_range,
                payload_range,
            }) => {
                let payload = &bytes[payload_range.clone()];

                occurrences.push(FieldOccurrence {
                    path: occurrence_path.clone(),
                    range: (base_offset + field_start)..(base_offset + full_range.end),
                });

                match field.runtime_field_type() {
                    RuntimeFieldType::Singular(RuntimeType::Message(child))
                    | RuntimeFieldType::Repeated(RuntimeType::Message(child)) => {
                        collect_occurrences(
                            &child,
                            payload,
                            base_offset + payload_range.start,
                            &mut occurrence_path.clone(),
                            occurrences,
                        );
                    }
                    _ => {}
                }
            }
            None => break,
        }
    }
}

enum ParsedField {
    Value(Range<usize>),
    LengthDelimited {
        full_range: Range<usize>,
        payload_range: Range<usize>,
    },
}

fn parse_field(bytes: &[u8], offset: &mut usize, wire_type: u8) -> Option<ParsedField> {
    match wire_type {
        0 => {
            let start = *offset;
            read_varint(bytes, offset)?;
            Some(ParsedField::Value(start..*offset))
        }
        1 => {
            let start = *offset;
            *offset = offset.checked_add(8)?;
            if *offset > bytes.len() {
                return None;
            }
            Some(ParsedField::Value(start..*offset))
        }
        2 => {
            let length_start = *offset;
            let length = read_varint(bytes, offset)? as usize;
            let payload_start = *offset;
            *offset = offset.checked_add(length)?;
            if *offset > bytes.len() {
                return None;
            }
            Some(ParsedField::LengthDelimited {
                full_range: length_start..*offset,
                payload_range: payload_start..*offset,
            })
        }
        5 => {
            let start = *offset;
            *offset = offset.checked_add(4)?;
            if *offset > bytes.len() {
                return None;
            }
            Some(ParsedField::Value(start..*offset))
        }
        _ => None,
    }
}

fn skip_unknown_field(bytes: &[u8], offset: &mut usize, wire_type: u8) -> Option<()> {
    match parse_field(bytes, offset, wire_type)? {
        ParsedField::Value(_) | ParsedField::LengthDelimited { .. } => Some(()),
    }
}

fn read_varint(bytes: &[u8], offset: &mut usize) -> Option<u64> {
    let mut result = 0_u64;
    let mut shift = 0_u32;

    while *offset < bytes.len() && shift < 64 {
        let byte = bytes[*offset];
        *offset += 1;
        result |= u64::from(byte & 0x7f) << shift;

        if byte & 0x80 == 0 {
            return Some(result);
        }

        shift += 7;
    }

    None
}

fn byte_offset_for_cursor(json: &str, cursor: (usize, usize)) -> Option<usize> {
    let mut lines = json.split('\n');
    let mut offset = 0;

    for _ in 0..cursor.0 {
        offset += lines.next()?.len() + 1;
    }

    let line = lines.next().unwrap_or_default();
    let column_offset = line
        .char_indices()
        .nth(cursor.1)
        .map(|(index, _)| index)
        .unwrap_or(line.len());

    Some(offset + column_offset)
}

#[derive(Debug, Clone)]
enum JsonNode {
    Object {
        span: Range<usize>,
        entries: Vec<JsonEntry>,
    },
    Array {
        span: Range<usize>,
        items: Vec<JsonNode>,
    },
    Scalar {
        span: Range<usize>,
    },
}

impl JsonNode {
    fn span(&self) -> &Range<usize> {
        match self {
            JsonNode::Object { span, .. }
            | JsonNode::Array { span, .. }
            | JsonNode::Scalar { span, .. } => span,
        }
    }

    fn path_at(&self, offset: usize) -> Option<FieldPath> {
        if !self.span().contains(&offset) && offset != self.span().end {
            return None;
        }

        match self {
            JsonNode::Object { entries, .. } => {
                for entry in entries {
                    if entry.contains(offset) {
                        let mut path = vec![FieldPathSegment::Field(entry.key.clone())];
                        if let Some(child_path) = entry.value.path_at(offset) {
                            path.extend(child_path);
                        }
                        return Some(path);
                    }
                }

                Some(Vec::new())
            }
            JsonNode::Array { items, .. } => {
                for (index, item) in items.iter().enumerate() {
                    if item.span().contains(&offset) || offset == item.span().end {
                        let mut path = vec![FieldPathSegment::Index(index)];
                        if let Some(child_path) = item.path_at(offset) {
                            path.extend(child_path);
                        }
                        return Some(path);
                    }
                }

                Some(Vec::new())
            }
            JsonNode::Scalar { .. } => Some(Vec::new()),
        }
    }
}

#[derive(Debug, Clone)]
struct JsonEntry {
    key: String,
    key_span: Range<usize>,
    value: JsonNode,
}

impl JsonEntry {
    fn contains(&self, offset: usize) -> bool {
        self.key_span.contains(&offset)
            || offset == self.key_span.end
            || self.value.span().contains(&offset)
            || offset == self.value.span().end
    }
}

struct JsonParser<'a> {
    input: &'a str,
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
            offset: 0,
        }
    }

    fn parse(mut self) -> Result<JsonNode, ()> {
        self.skip_whitespace();
        let node = self.parse_value()?;
        self.skip_whitespace();
        if self.offset == self.bytes.len() {
            Ok(node)
        } else {
            Err(())
        }
    }

    fn parse_value(&mut self) -> Result<JsonNode, ()> {
        self.skip_whitespace();
        let start = self.offset;

        match self.peek_byte() {
            Some(b'{') => self.parse_object(start),
            Some(b'[') => self.parse_array(start),
            Some(b'"') => {
                self.parse_string()?;
                Ok(JsonNode::Scalar {
                    span: start..self.offset,
                })
            }
            Some(b'-' | b'0'..=b'9') => {
                self.parse_number()?;
                Ok(JsonNode::Scalar {
                    span: start..self.offset,
                })
            }
            Some(b't') => {
                self.expect_bytes(b"true")?;
                Ok(JsonNode::Scalar {
                    span: start..self.offset,
                })
            }
            Some(b'f') => {
                self.expect_bytes(b"false")?;
                Ok(JsonNode::Scalar {
                    span: start..self.offset,
                })
            }
            Some(b'n') => {
                self.expect_bytes(b"null")?;
                Ok(JsonNode::Scalar {
                    span: start..self.offset,
                })
            }
            _ => Err(()),
        }
    }

    fn parse_object(&mut self, start: usize) -> Result<JsonNode, ()> {
        self.expect_byte(b'{')?;
        let mut entries = Vec::new();
        self.skip_whitespace();

        if self.peek_byte() == Some(b'}') {
            self.offset += 1;
            return Ok(JsonNode::Object {
                span: start..self.offset,
                entries,
            });
        }

        loop {
            self.skip_whitespace();
            let key_start = self.offset;
            let key = self.parse_string()?;
            let key_span = key_start..self.offset;
            self.skip_whitespace();
            self.expect_byte(b':')?;
            self.skip_whitespace();
            let value = self.parse_value()?;

            entries.push(JsonEntry {
                key,
                key_span,
                value,
            });

            self.skip_whitespace();
            match self.peek_byte() {
                Some(b',') => {
                    self.offset += 1;
                }
                Some(b'}') => {
                    self.offset += 1;
                    break;
                }
                _ => return Err(()),
            }
        }

        Ok(JsonNode::Object {
            span: start..self.offset,
            entries,
        })
    }

    fn parse_array(&mut self, start: usize) -> Result<JsonNode, ()> {
        self.expect_byte(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();

        if self.peek_byte() == Some(b']') {
            self.offset += 1;
            return Ok(JsonNode::Array {
                span: start..self.offset,
                items,
            });
        }

        loop {
            self.skip_whitespace();
            items.push(self.parse_value()?);
            self.skip_whitespace();

            match self.peek_byte() {
                Some(b',') => {
                    self.offset += 1;
                }
                Some(b']') => {
                    self.offset += 1;
                    break;
                }
                _ => return Err(()),
            }
        }

        Ok(JsonNode::Array {
            span: start..self.offset,
            items,
        })
    }

    fn parse_string(&mut self) -> Result<String, ()> {
        let start = self.offset;
        self.expect_byte(b'"')?;

        while let Some(byte) = self.peek_byte() {
            match byte {
                b'\\' => {
                    self.offset += 1;
                    let escape = self.peek_byte().ok_or(())?;
                    self.offset += 1;
                    if escape == b'u' {
                        for _ in 0..4 {
                            let hex = self.peek_byte().ok_or(())?;
                            if !hex.is_ascii_hexdigit() {
                                return Err(());
                            }
                            self.offset += 1;
                        }
                    }
                }
                b'"' => {
                    self.offset += 1;
                    let slice = &self.input[start..self.offset];
                    return serde_json::from_str(slice).map_err(|_| ());
                }
                _ => {
                    self.offset += 1;
                }
            }
        }

        Err(())
    }

    fn parse_number(&mut self) -> Result<(), ()> {
        if self.peek_byte() == Some(b'-') {
            self.offset += 1;
        }

        match self.peek_byte() {
            Some(b'0') => {
                self.offset += 1;
            }
            Some(b'1'..=b'9') => {
                self.offset += 1;
                while matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                    self.offset += 1;
                }
            }
            _ => return Err(()),
        }

        if self.peek_byte() == Some(b'.') {
            self.offset += 1;
            if !matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                return Err(());
            }

            while matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                self.offset += 1;
            }
        }

        if matches!(self.peek_byte(), Some(b'e' | b'E')) {
            self.offset += 1;
            if matches!(self.peek_byte(), Some(b'+' | b'-')) {
                self.offset += 1;
            }

            if !matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                return Err(());
            }

            while matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                self.offset += 1;
            }
        }

        Ok(())
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek_byte(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.offset += 1;
        }
    }

    fn expect_byte(&mut self, expected: u8) -> Result<(), ()> {
        match self.peek_byte() {
            Some(byte) if byte == expected => {
                self.offset += 1;
                Ok(())
            }
            _ => Err(()),
        }
    }

    fn expect_bytes(&mut self, expected: &[u8]) -> Result<(), ()> {
        if self.bytes.get(self.offset..self.offset + expected.len()) == Some(expected) {
            self.offset += expected.len();
            Ok(())
        } else {
            Err(())
        }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use protobuf::{
        EnumOrUnknown, Message as _, MessageField, MessageFull, SpecialFields,
        well_known_types::timestamp::Timestamp,
    };
    use protogen::system_event::{
        SystemEvent,
        system_event::{Event as SystemEventVariant, MouseButton, MouseDown},
    };

    use super::*;
    use crate::inspector::{InputFormat, load_inspector};

    fn sample_message() -> SystemEvent {
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
    }

    fn json_cursor(json: &str, needle: &str) -> (usize, usize) {
        let (line, column) = json
            .lines()
            .enumerate()
            .find_map(|(line, text)| text.find(needle).map(|column| (line, column)))
            .unwrap();

        (line, column)
    }

    fn descriptor() -> MessageDescriptor {
        SystemEvent::descriptor()
    }

    fn schema_path() -> camino::Utf8PathBuf {
        camino::Utf8PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../protogen/proto/system-event.proto"
        ))
    }

    #[test]
    fn resolves_json_cursor_to_nested_field_path() {
        let inspector = load_inspector(
            schema_path().as_ref(),
            Some("SystemEvent"),
            &sample_message().write_to_bytes().unwrap(),
            InputFormat::Binary,
        )
        .unwrap();
        let json = inspector.canonical_json().unwrap();

        let cursor = json_cursor(&json, "\"seconds\"");
        let path = selected_path_for_json_cursor(&descriptor(), &json, cursor).unwrap();

        assert_eq!(
            path,
            vec![
                FieldPathSegment::Field("timestamp".to_owned()),
                FieldPathSegment::Field("seconds".to_owned()),
            ]
        );
    }

    #[test]
    fn highlights_bytes_for_selected_scalar_field() {
        let message = sample_message();
        let bytes = message.write_to_bytes().unwrap();
        let highlighted = highlighted_byte_indices(
            &descriptor(),
            &bytes,
            &[FieldPathSegment::Field("reason".to_owned())],
        );

        let selected_bytes = highlighted
            .iter()
            .map(|index| bytes[*index])
            .collect::<Vec<_>>();

        assert_eq!(
            selected_bytes,
            SystemEvent {
                reason: Some("user clicked".to_owned()),
                ..Default::default()
            }
            .write_to_bytes()
            .unwrap()
        );
    }

    #[test]
    fn protobuf_lines_include_nested_paths() {
        let message = sample_message();
        let lines = protobuf_lines(&descriptor(), &message);
        let selected = vec![
            FieldPathSegment::Field("timestamp".to_owned()),
            FieldPathSegment::Field("seconds".to_owned()),
        ];

        let highlighted = lines
            .into_iter()
            .filter(|line| related_path(&selected, &line.path))
            .map(|line| line.text)
            .collect::<BTreeSet<_>>();

        assert!(highlighted.contains("timestamp {"));
        assert!(highlighted.contains("  seconds: 1234567"));
        assert!(highlighted.contains("}"));
    }
}
