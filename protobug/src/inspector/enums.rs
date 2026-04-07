use protobuf::{
    MessageDyn,
    reflect::{
        MessageDescriptor, ReflectFieldRef, ReflectValueBox, ReflectValueRef, RuntimeFieldType,
        RuntimeType,
    },
};

use crate::selection;

use super::EnumSelection;

pub(super) fn omitted_default_enum_hint(
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

pub(super) fn enum_selection(
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

pub(super) fn cycle_enum_variant(
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
