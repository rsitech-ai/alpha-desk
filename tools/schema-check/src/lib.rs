#![forbid(unsafe_code)]

use prost::Message;
use prost_types::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    FileDescriptorSet,
};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    path::Path,
};

pub const MAX_DESCRIPTOR_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SchemaCheckError {
    #[error("{label} descriptor is {actual} bytes and exceeds the {maximum}-byte input limit")]
    TooLarge {
        label: String,
        actual: usize,
        maximum: usize,
    },
    #[error("failed to decode {label} descriptor: {source}")]
    Decode {
        label: String,
        #[source]
        source: prost::DecodeError,
    },
    #[error("failed to read descriptor {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("invalid descriptor: {0}")]
    InvalidDescriptor(String),
    #[error("schema incompatibility: {0}")]
    Incompatible(String),
}

pub fn decode_descriptor_set(
    label: &str,
    bytes: &[u8],
) -> Result<FileDescriptorSet, SchemaCheckError> {
    if bytes.len() > MAX_DESCRIPTOR_BYTES {
        return Err(SchemaCheckError::TooLarge {
            label: label.to_owned(),
            actual: bytes.len(),
            maximum: MAX_DESCRIPTOR_BYTES,
        });
    }
    FileDescriptorSet::decode(bytes).map_err(|source| SchemaCheckError::Decode {
        label: label.to_owned(),
        source,
    })
}

pub fn read_descriptor_file(
    label: &str,
    path: &Path,
) -> Result<FileDescriptorSet, SchemaCheckError> {
    let path_display = path.display().to_string();
    let metadata = path.metadata().map_err(|source| SchemaCheckError::Read {
        path: path_display.clone(),
        source,
    })?;
    let metadata_length = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    if metadata_length > MAX_DESCRIPTOR_BYTES {
        return Err(SchemaCheckError::TooLarge {
            label: label.to_owned(),
            actual: metadata_length,
            maximum: MAX_DESCRIPTOR_BYTES,
        });
    }

    let file = File::open(path).map_err(|source| SchemaCheckError::Read {
        path: path_display.clone(),
        source,
    })?;
    let read_limit = u64::try_from(MAX_DESCRIPTOR_BYTES)
        .map_err(|_| {
            SchemaCheckError::InvalidDescriptor(
                "descriptor size limit does not fit into u64".to_owned(),
            )
        })?
        .saturating_add(1);
    let mut bytes = Vec::with_capacity(metadata_length);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|source| SchemaCheckError::Read {
            path: path_display,
            source,
        })?;
    decode_descriptor_set(label, &bytes)
}

pub fn check_file_descriptor_sets(
    baseline: &FileDescriptorSet,
    current: &FileDescriptorSet,
) -> Result<(), SchemaCheckError> {
    validate_descriptor_set("baseline", baseline)?;
    validate_descriptor_set("current", current)?;

    let current_files = index_files(current)?;
    for baseline_file in &baseline.file {
        let baseline_name = required_text(baseline_file.name.as_deref(), "baseline file name")?;
        let current_file = current_files.get(baseline_name).copied().ok_or_else(|| {
            SchemaCheckError::Incompatible(format!(
                "semantic-major/package drift: baseline file {baseline_name} is absent from the current descriptor set"
            ))
        })?;
        compare_file(baseline_file, current_file)?;
    }
    Ok(())
}

fn validate_descriptor_set(label: &str, set: &FileDescriptorSet) -> Result<(), SchemaCheckError> {
    if set.file.is_empty() {
        return Err(SchemaCheckError::InvalidDescriptor(format!(
            "{label} descriptor set contains no files"
        )));
    }
    let mut names = BTreeMap::new();
    for file in &set.file {
        let name = required_text(file.name.as_deref(), "file name")?;
        let package = required_text(file.package.as_deref(), "file package")?;
        if names.insert(name, ()).is_some() {
            return Err(SchemaCheckError::InvalidDescriptor(format!(
                "{label} descriptor repeats file {name}"
            )));
        }
        validate_semantic_major(name, package)?;
    }
    Ok(())
}

fn validate_semantic_major(path: &str, package: &str) -> Result<(), SchemaCheckError> {
    let path_major = version_component(path.split('/')).ok_or_else(|| {
        SchemaCheckError::InvalidDescriptor(format!(
            "semantic-major/package drift: file path {path} has no vN component"
        ))
    })?;
    let package_major = version_component(package.split('.')).ok_or_else(|| {
        SchemaCheckError::InvalidDescriptor(format!(
            "semantic-major/package drift: package {package} has no vN component"
        ))
    })?;
    if path_major != package_major {
        return Err(SchemaCheckError::Incompatible(format!(
            "semantic-major/package drift: file {path} declares package {package}"
        )));
    }
    Ok(())
}

fn version_component<'a>(parts: impl Iterator<Item = &'a str>) -> Option<u64> {
    parts
        .filter_map(|part| part.strip_prefix('v'))
        .find_map(|digits| {
            (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| digits.parse::<u64>().ok())
                .flatten()
        })
}

fn index_files(
    set: &FileDescriptorSet,
) -> Result<BTreeMap<&str, &FileDescriptorProto>, SchemaCheckError> {
    set.file
        .iter()
        .map(|file| required_text(file.name.as_deref(), "file name").map(|name| (name, file)))
        .collect()
}

fn compare_file(
    baseline: &FileDescriptorProto,
    current: &FileDescriptorProto,
) -> Result<(), SchemaCheckError> {
    let name = required_text(baseline.name.as_deref(), "baseline file name")?;
    let baseline_package = required_text(baseline.package.as_deref(), "baseline package")?;
    let current_package = required_text(current.package.as_deref(), "current package")?;
    if baseline_package != current_package {
        return Err(SchemaCheckError::Incompatible(format!(
            "semantic-major/package drift for {name}: {baseline_package} became {current_package}"
        )));
    }
    if baseline.syntax != current.syntax {
        return Err(SchemaCheckError::Incompatible(format!(
            "syntax changed for {name}: {:?} became {:?}",
            baseline.syntax, current.syntax
        )));
    }

    compare_messages(
        baseline_package,
        &baseline.message_type,
        &current.message_type,
    )?;
    compare_enums(baseline_package, &baseline.enum_type, &current.enum_type)
}

fn compare_messages(
    scope: &str,
    baseline: &[DescriptorProto],
    current: &[DescriptorProto],
) -> Result<(), SchemaCheckError> {
    let current_by_name = index_messages(current)?;
    for baseline_message in baseline {
        let name = required_text(baseline_message.name.as_deref(), "baseline message name")?;
        let qualified = format!("{scope}.{name}");
        let current_message = current_by_name.get(name).copied().ok_or_else(|| {
            SchemaCheckError::Incompatible(format!("removed message {qualified}"))
        })?;
        compare_fields(&qualified, &baseline_message.field, &current_message.field)?;
        compare_messages(
            &qualified,
            &baseline_message.nested_type,
            &current_message.nested_type,
        )?;
        compare_enums(
            &qualified,
            &baseline_message.enum_type,
            &current_message.enum_type,
        )?;
    }
    Ok(())
}

fn index_messages(
    messages: &[DescriptorProto],
) -> Result<BTreeMap<&str, &DescriptorProto>, SchemaCheckError> {
    messages
        .iter()
        .map(|message| {
            required_text(message.name.as_deref(), "message name").map(|name| (name, message))
        })
        .collect()
}

fn compare_fields(
    message: &str,
    baseline: &[FieldDescriptorProto],
    current: &[FieldDescriptorProto],
) -> Result<(), SchemaCheckError> {
    let current_by_name = index_fields_by_name(current)?;
    let current_by_number = index_fields_by_number(current)?;
    for baseline_field in baseline {
        let name = required_text(baseline_field.name.as_deref(), "baseline field name")?;
        let number = required_number(baseline_field.number, "baseline field number")?;

        let Some(current_field) = current_by_name.get(name).copied() else {
            if let Some(replacement) = current_by_number.get(&number) {
                let replacement_name =
                    required_text(replacement.name.as_deref(), "replacement field name")?;
                return Err(SchemaCheckError::Incompatible(format!(
                    "field number {number} in {message} was reused/name-replaced: {name} became {replacement_name}; deleted names and numbers must be reserved"
                )));
            }
            return Err(SchemaCheckError::Incompatible(format!(
                "removed field {message}.{name} number {number}; deleted names and numbers must be reserved"
            )));
        };

        let current_number = required_number(current_field.number, "current field number")?;
        if number != current_number {
            return Err(SchemaCheckError::Incompatible(format!(
                "renumbered field {message}.{name}: {number} became {current_number}; field numbers cannot change"
            )));
        }
        if baseline_field.r#type != current_field.r#type
            || baseline_field.type_name != current_field.type_name
        {
            return Err(SchemaCheckError::Incompatible(format!(
                "incompatible wire type for {message}.{name}: {:?}/{:?} became {:?}/{:?}",
                baseline_field.r#type,
                baseline_field.type_name,
                current_field.r#type,
                current_field.type_name
            )));
        }
        if baseline_field.label != current_field.label
            || baseline_field.proto3_optional != current_field.proto3_optional
        {
            return Err(SchemaCheckError::Incompatible(format!(
                "incompatible cardinality for {message}.{name}: {:?} became {:?}",
                baseline_field.label, current_field.label
            )));
        }
        if baseline_field.oneof_index != current_field.oneof_index {
            return Err(SchemaCheckError::Incompatible(format!(
                "incompatible oneof membership for {message}.{name}"
            )));
        }
    }
    Ok(())
}

fn index_fields_by_name(
    fields: &[FieldDescriptorProto],
) -> Result<BTreeMap<&str, &FieldDescriptorProto>, SchemaCheckError> {
    fields
        .iter()
        .map(|field| required_text(field.name.as_deref(), "field name").map(|name| (name, field)))
        .collect()
}

fn index_fields_by_number(
    fields: &[FieldDescriptorProto],
) -> Result<BTreeMap<i32, &FieldDescriptorProto>, SchemaCheckError> {
    fields
        .iter()
        .map(|field| required_number(field.number, "field number").map(|number| (number, field)))
        .collect()
}

fn compare_enums(
    scope: &str,
    baseline: &[EnumDescriptorProto],
    current: &[EnumDescriptorProto],
) -> Result<(), SchemaCheckError> {
    let current_by_name = index_enums(current)?;
    for baseline_enum in baseline {
        let name = required_text(baseline_enum.name.as_deref(), "baseline enum name")?;
        let qualified = format!("{scope}.{name}");
        let current_enum = current_by_name
            .get(name)
            .copied()
            .ok_or_else(|| SchemaCheckError::Incompatible(format!("removed enum {qualified}")))?;
        compare_enum_values(&qualified, baseline_enum, current_enum)?;
    }
    Ok(())
}

fn index_enums(
    enums: &[EnumDescriptorProto],
) -> Result<BTreeMap<&str, &EnumDescriptorProto>, SchemaCheckError> {
    enums
        .iter()
        .map(|enumeration| {
            required_text(enumeration.name.as_deref(), "enum name").map(|name| (name, enumeration))
        })
        .collect()
}

fn compare_enum_values(
    enumeration: &str,
    baseline: &EnumDescriptorProto,
    current: &EnumDescriptorProto,
) -> Result<(), SchemaCheckError> {
    let current_by_name = current
        .value
        .iter()
        .map(|value| {
            required_text(value.name.as_deref(), "enum value name").map(|name| (name, value))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let current_by_number = current
        .value
        .iter()
        .map(|value| {
            required_number(value.number, "enum value number").map(|number| (number, value))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    for baseline_value in &baseline.value {
        let name = required_text(baseline_value.name.as_deref(), "baseline enum value name")?;
        let number = required_number(baseline_value.number, "baseline enum value number")?;
        let Some(current_value) = current_by_name.get(name).copied() else {
            if let Some(replacement) = current_by_number.get(&number) {
                let replacement_name =
                    required_text(replacement.name.as_deref(), "replacement enum value name")?;
                return Err(SchemaCheckError::Incompatible(format!(
                    "enum number {number} in {enumeration} was reused/name-replaced: {name} became {replacement_name}; deleted names and numbers must be reserved"
                )));
            }
            return Err(SchemaCheckError::Incompatible(format!(
                "removed enum value {enumeration}.{name} number {number}; deleted names and numbers must be reserved"
            )));
        };
        let current_number = required_number(current_value.number, "current enum value number")?;
        if number != current_number {
            return Err(SchemaCheckError::Incompatible(format!(
                "renumbered enum value {enumeration}.{name}: {number} became {current_number}; enum numbers cannot change"
            )));
        }
    }
    Ok(())
}

fn required_text<'a>(
    value: Option<&'a str>,
    description: &str,
) -> Result<&'a str, SchemaCheckError> {
    value
        .filter(|value| !value.is_empty())
        .ok_or_else(|| SchemaCheckError::InvalidDescriptor(format!("missing {description}")))
}

fn required_number(value: Option<i32>, description: &str) -> Result<i32, SchemaCheckError> {
    value.ok_or_else(|| SchemaCheckError::InvalidDescriptor(format!("missing {description}")))
}
