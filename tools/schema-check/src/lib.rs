#![forbid(unsafe_code)]

use prost::Message;
use prost_types::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    FileDescriptorSet, ServiceDescriptorProto,
    field_descriptor_proto::{Label, Type},
};
use std::{
    collections::{BTreeMap, BTreeSet},
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
    let mut names = BTreeSet::new();
    for file in &set.file {
        let name = required_text(file.name.as_deref(), "file name")?;
        let package = required_text(file.package.as_deref(), "file package")?;
        if !names.insert(name) {
            return Err(SchemaCheckError::InvalidDescriptor(format!(
                "{label} descriptor repeats file {name}"
            )));
        }
        validate_semantic_major(name, package)?;
    }

    let mut symbols = Symbols::default();
    for file in &set.file {
        let file_name = required_text(file.name.as_deref(), "file name")?;
        let package = required_text(file.package.as_deref(), "file package")?;
        collect_symbols(
            file_name,
            package,
            &file.message_type,
            &file.enum_type,
            &mut symbols,
        )?;
    }
    let files = index_files(set)?;
    for file in &set.file {
        validate_file(label, file, &names, &files, &symbols)?;
    }
    Ok(())
}

#[derive(Default)]
struct Symbols {
    messages: BTreeMap<String, String>,
    enums: BTreeMap<String, String>,
    enum_values: BTreeMap<String, String>,
    map_entries: BTreeMap<String, String>,
}

fn collect_symbols(
    file_name: &str,
    scope: &str,
    messages: &[DescriptorProto],
    enums: &[EnumDescriptorProto],
    symbols: &mut Symbols,
) -> Result<(), SchemaCheckError> {
    let mut local_names = BTreeSet::new();
    for message in messages {
        let name = required_text(message.name.as_deref(), "message name")?;
        if !local_names.insert(name) {
            return Err(SchemaCheckError::InvalidDescriptor(format!(
                "duplicate nested type name {scope}.{name}"
            )));
        }
        let qualified = format!(".{scope}.{name}");
        if symbols.enum_values.contains_key(&qualified) {
            return invalid(format!("enum value namespace collision for {qualified}"));
        }
        if symbols
            .messages
            .insert(qualified.clone(), file_name.to_owned())
            .is_some()
        {
            return Err(SchemaCheckError::InvalidDescriptor(format!(
                "duplicate message symbol {qualified}"
            )));
        }
        if message
            .options
            .as_ref()
            .and_then(|options| options.map_entry)
            .unwrap_or(false)
        {
            symbols
                .map_entries
                .insert(qualified.clone(), format!(".{scope}"));
        }
        collect_symbols(
            file_name,
            &format!("{scope}.{name}"),
            &message.nested_type,
            &message.enum_type,
            symbols,
        )?;
    }
    for enumeration in enums {
        let name = required_text(enumeration.name.as_deref(), "enum name")?;
        if !local_names.insert(name) {
            return Err(SchemaCheckError::InvalidDescriptor(format!(
                "duplicate nested type name {scope}.{name}"
            )));
        }
        let qualified = format!(".{scope}.{name}");
        if symbols.enum_values.contains_key(&qualified) {
            return invalid(format!("enum value namespace collision for {qualified}"));
        }
        if symbols
            .enums
            .insert(qualified.clone(), file_name.to_owned())
            .is_some()
        {
            return Err(SchemaCheckError::InvalidDescriptor(format!(
                "duplicate enum symbol {qualified}"
            )));
        }
        for value in &enumeration.value {
            let value_name = required_text(value.name.as_deref(), "enum value name")?;
            let value_qualified = format!(".{scope}.{value_name}");
            if symbols.messages.contains_key(&value_qualified)
                || symbols.enums.contains_key(&value_qualified)
            {
                return invalid(format!(
                    "enum value namespace collision for {value_qualified}"
                ));
            }
            if let Some(owner) = symbols
                .enum_values
                .insert(value_qualified.clone(), qualified.clone())
                && owner != qualified
            {
                return invalid(format!(
                    "enum value namespace collision for {value_qualified}"
                ));
            }
        }
    }
    Ok(())
}

fn validate_file(
    label: &str,
    file: &FileDescriptorProto,
    file_names: &BTreeSet<&str>,
    files: &BTreeMap<&str, &FileDescriptorProto>,
    symbols: &Symbols,
) -> Result<(), SchemaCheckError> {
    let name = required_text(file.name.as_deref(), "file name")?;
    let package = required_text(file.package.as_deref(), "file package")?;
    let syntax = required_text(file.syntax.as_deref(), "file syntax")?;
    if syntax != "proto2" && syntax != "proto3" {
        return invalid(format!(
            "{label} file {name} has unsupported syntax {syntax}"
        ));
    }

    let mut dependencies = BTreeSet::new();
    for dependency in &file.dependency {
        if dependency.is_empty() || !dependencies.insert(dependency.as_str()) {
            return invalid(format!(
                "{label} file {name} has duplicate or empty dependency"
            ));
        }
        if !file_names.contains(dependency.as_str()) {
            return invalid(format!(
                "{label} file {name} references unknown dependency {dependency}"
            ));
        }
    }
    for index in file
        .public_dependency
        .iter()
        .chain(file.weak_dependency.iter())
    {
        let valid = usize::try_from(*index)
            .ok()
            .is_some_and(|index| index < file.dependency.len());
        if !valid {
            return invalid(format!(
                "{label} file {name} has invalid dependency index {index}"
            ));
        }
    }

    let visible_files = visible_files(name, file, files)?;
    validate_enum_value_namespace(package, &file.enum_type)?;
    for message in &file.message_type {
        validate_message(package, message, syntax, symbols, &visible_files, false)?;
    }
    for enumeration in &file.enum_type {
        validate_enum(package, enumeration, syntax)?;
    }
    validate_services(package, &file.service, symbols, &visible_files)?;
    Ok(())
}

fn visible_files(
    file_name: &str,
    file: &FileDescriptorProto,
    files: &BTreeMap<&str, &FileDescriptorProto>,
) -> Result<BTreeSet<String>, SchemaCheckError> {
    let mut visible = BTreeSet::from([file_name.to_owned()]);
    for dependency in &file.dependency {
        visible.insert(dependency.clone());
        add_public_imports(dependency, files, &mut visible, &mut BTreeSet::new())?;
    }
    Ok(visible)
}

fn add_public_imports(
    file_name: &str,
    files: &BTreeMap<&str, &FileDescriptorProto>,
    visible: &mut BTreeSet<String>,
    visiting: &mut BTreeSet<String>,
) -> Result<(), SchemaCheckError> {
    if !visiting.insert(file_name.to_owned()) {
        return Ok(());
    }
    let file = files.get(file_name).copied().ok_or_else(|| {
        SchemaCheckError::InvalidDescriptor(format!(
            "public import closure references unknown dependency {file_name}"
        ))
    })?;
    for index in &file.public_dependency {
        let dependency = usize::try_from(*index)
            .ok()
            .and_then(|index| file.dependency.get(index))
            .ok_or_else(|| {
                SchemaCheckError::InvalidDescriptor(format!(
                    "file {file_name} has invalid public dependency index {index}"
                ))
            })?;
        visible.insert(dependency.clone());
        add_public_imports(dependency, files, visible, visiting)?;
    }
    visiting.remove(file_name);
    Ok(())
}

fn validate_message(
    scope: &str,
    message: &DescriptorProto,
    syntax: &str,
    symbols: &Symbols,
    visible_files: &BTreeSet<String>,
    nested: bool,
) -> Result<(), SchemaCheckError> {
    let name = required_text(message.name.as_deref(), "message name")?;
    let qualified = format!("{scope}.{name}");
    let map_entry = message
        .options
        .as_ref()
        .and_then(|options| options.map_entry)
        .unwrap_or(false);
    if map_entry && !nested {
        return invalid(format!("map entry {qualified} must be nested"));
    }

    let mut declaration_names = BTreeSet::new();
    let mut oneof_names = BTreeSet::new();
    for oneof in &message.oneof_decl {
        let oneof_name = required_text(oneof.name.as_deref(), "oneof name")?;
        if !oneof_names.insert(oneof_name) {
            return invalid(format!("duplicate oneof name {qualified}.{oneof_name}"));
        }
        if !declaration_names.insert(oneof_name) {
            return invalid(format!(
                "message namespace collision for {qualified}.{oneof_name}"
            ));
        }
    }

    let reserved_names = validate_reserved_names(&qualified, "field", &message.reserved_name)?;
    let reserved_ranges = validate_message_reserved_ranges(&qualified, &message.reserved_range)?;

    let mut field_names = BTreeSet::new();
    let mut field_numbers = BTreeSet::new();
    for field in message.field.iter().chain(message.extension.iter()) {
        let field_name = required_text(field.name.as_deref(), "field name")?;
        let number = required_number(field.number, "field number")?;
        if !field_names.insert(field_name) {
            return invalid(format!("duplicate field name {qualified}.{field_name}"));
        }
        if !declaration_names.insert(field_name) {
            return invalid(format!(
                "message namespace collision for {qualified}.{field_name}"
            ));
        }
        if !field_numbers.insert(number) {
            return invalid(format!("duplicate field number {number} in {qualified}"));
        }
        validate_field_number(number, &qualified)?;
        if reserved_names.contains(field_name) {
            return invalid(format!(
                "reserved field name {qualified}.{field_name} was reused"
            ));
        }
        if range_contains_message(&reserved_ranges, number) {
            return invalid(format!(
                "reserved field number {number} in {qualified} was reused"
            ));
        }
        validate_field(
            &qualified,
            field,
            syntax,
            message.oneof_decl.len(),
            symbols,
            visible_files,
        )?;
    }

    let mut nested_names = BTreeSet::new();
    for child in &message.nested_type {
        let child_name = required_text(child.name.as_deref(), "nested message name")?;
        if !nested_names.insert(child_name) {
            return invalid(format!(
                "duplicate nested type name {qualified}.{child_name}"
            ));
        }
        if !declaration_names.insert(child_name) {
            return invalid(format!(
                "message namespace collision for {qualified}.{child_name}"
            ));
        }
    }
    for enumeration in &message.enum_type {
        let enum_name = required_text(enumeration.name.as_deref(), "nested enum name")?;
        if !nested_names.insert(enum_name) {
            return invalid(format!(
                "duplicate nested type name {qualified}.{enum_name}"
            ));
        }
        if !declaration_names.insert(enum_name) {
            return invalid(format!(
                "message namespace collision for {qualified}.{enum_name}"
            ));
        }
    }
    let mut enum_value_owners = BTreeMap::new();
    for (enum_index, enumeration) in message.enum_type.iter().enumerate() {
        for value in &enumeration.value {
            let value_name = required_text(value.name.as_deref(), "enum value name")?;
            if enum_value_owners.get(value_name) == Some(&enum_index) {
                continue;
            }
            if enum_value_owners.insert(value_name, enum_index).is_some()
                || !declaration_names.insert(value_name)
            {
                return invalid(format!(
                    "enum value namespace collision for {qualified}.{value_name}"
                ));
            }
        }
    }

    validate_oneofs(&qualified, message)?;
    validate_parent_map_fields(&qualified, message, symbols)?;

    if map_entry {
        validate_map_entry(&qualified, message)?;
    }
    for child in &message.nested_type {
        validate_message(&qualified, child, syntax, symbols, visible_files, true)?;
    }
    for enumeration in &message.enum_type {
        validate_enum(&qualified, enumeration, syntax)?;
    }
    Ok(())
}

fn validate_field(
    message: &str,
    field: &FieldDescriptorProto,
    syntax: &str,
    oneof_count: usize,
    symbols: &Symbols,
    visible_files: &BTreeSet<String>,
) -> Result<(), SchemaCheckError> {
    let name = required_text(field.name.as_deref(), "field name")?;
    let label_value = required_number(field.label, "field label")?;
    let label = Label::try_from(label_value).map_err(|_| {
        SchemaCheckError::InvalidDescriptor(format!("invalid field label for {message}.{name}"))
    })?;
    if syntax == "proto3" && label == Label::Required {
        return invalid(format!(
            "required field {message}.{name} is invalid in proto3"
        ));
    }
    if field.proto3_optional.unwrap_or(false) && syntax != "proto3" {
        return invalid(format!(
            "proto3 optional field {message}.{name} is invalid in {syntax}"
        ));
    }
    let type_value = required_number(field.r#type, "field type")?;
    let field_type = Type::try_from(type_value).map_err(|_| {
        SchemaCheckError::InvalidDescriptor(format!("invalid field type for {message}.{name}"))
    })?;

    if let Some(index) = field.oneof_index {
        let valid = usize::try_from(index)
            .ok()
            .is_some_and(|index| index < oneof_count);
        if !valid {
            return invalid(format!("invalid oneof index {index} for {message}.{name}"));
        }
        if label == Label::Repeated {
            return invalid(format!(
                "repeated field {message}.{name} cannot belong to a oneof"
            ));
        }
    } else if field.proto3_optional.unwrap_or(false) {
        return invalid(format!(
            "proto3 optional field {message}.{name} is missing synthetic oneof membership"
        ));
    }
    if field.proto3_optional.unwrap_or(false) && label != Label::Optional {
        return invalid(format!(
            "proto3 optional field {message}.{name} must have optional cardinality"
        ));
    }

    match field_type {
        Type::Message | Type::Group => {
            let reference = required_text(field.type_name.as_deref(), "message type reference")?;
            let Some(origin) = symbols.messages.get(reference) else {
                return invalid(format!(
                    "unknown type reference {reference} for {message}.{name}"
                ));
            };
            validate_reference_visibility(reference, origin, message, name, visible_files)?;
            if let Some(parent) = symbols.map_entries.get(reference)
                && (parent != &format!(".{message}")
                    || label != Label::Repeated
                    || field.oneof_index.is_some()
                    || field.proto3_optional.unwrap_or(false))
            {
                return invalid(format!(
                    "map entry {reference} must be used by one legal repeated map field in {parent}"
                ));
            }
        }
        Type::Enum => {
            let reference = required_text(field.type_name.as_deref(), "enum type reference")?;
            let Some(origin) = symbols.enums.get(reference) else {
                return invalid(format!(
                    "unknown type reference {reference} for {message}.{name}"
                ));
            };
            validate_reference_visibility(reference, origin, message, name, visible_files)?;
        }
        _ if field
            .type_name
            .as_deref()
            .is_some_and(|value| !value.is_empty()) =>
        {
            return invalid(format!(
                "scalar field {message}.{name} has an unexpected type reference"
            ));
        }
        _ => {}
    }
    Ok(())
}

fn validate_reference_visibility(
    reference: &str,
    origin: &str,
    owner: &str,
    member: &str,
    visible_files: &BTreeSet<String>,
) -> Result<(), SchemaCheckError> {
    if !visible_files.contains(origin) {
        return invalid(format!(
            "missing import for cross-file type reference {reference} used by {owner}.{member}; symbol is defined in {origin}"
        ));
    }
    Ok(())
}

fn validate_oneofs(qualified: &str, message: &DescriptorProto) -> Result<(), SchemaCheckError> {
    let mut first_synthetic = None;
    for (index, _) in message.oneof_decl.iter().enumerate() {
        let members = message
            .field
            .iter()
            .filter(|field| field.oneof_index == i32::try_from(index).ok())
            .collect::<Vec<_>>();
        if members.is_empty() {
            return invalid(format!(
                "oneof {index} in {qualified} must contain at least one field"
            ));
        }
        let synthetic = members
            .iter()
            .any(|field| field.proto3_optional == Some(true));
        if synthetic {
            if members.len() != 1 || members[0].proto3_optional != Some(true) {
                return invalid(format!(
                    "synthetic proto3 optional oneof {index} in {qualified} must contain exactly one proto3 optional field"
                ));
            }
            first_synthetic.get_or_insert(index);
        } else if let Some(synthetic_index) = first_synthetic {
            return invalid(format!(
                "synthetic proto3 optional oneof {synthetic_index} in {qualified} must appear after all real oneofs"
            ));
        }
    }
    Ok(())
}

fn validate_parent_map_fields(
    qualified: &str,
    message: &DescriptorProto,
    symbols: &Symbols,
) -> Result<(), SchemaCheckError> {
    for child in &message.nested_type {
        let is_map_entry = child
            .options
            .as_ref()
            .and_then(|options| options.map_entry)
            .unwrap_or(false);
        if !is_map_entry {
            continue;
        }
        let child_name = required_text(child.name.as_deref(), "map entry name")?;
        let reference = format!(".{qualified}.{child_name}");
        let references = message
            .field
            .iter()
            .filter(|field| field.type_name.as_deref() == Some(reference.as_str()))
            .count();
        if references != 1 {
            return invalid(format!(
                "map entry {reference} must be referenced by exactly one legal repeated map field; found {references}"
            ));
        }
        if !symbols.map_entries.contains_key(&reference) {
            return invalid(format!("map entry symbol {reference} was not indexed"));
        }
    }
    Ok(())
}

fn validate_field_number(number: i32, message: &str) -> Result<(), SchemaCheckError> {
    if !(1..=536_870_911).contains(&number) || (19_000..=19_999).contains(&number) {
        return invalid(format!("invalid field number {number} in {message}"));
    }
    Ok(())
}

fn validate_map_entry(qualified: &str, message: &DescriptorProto) -> Result<(), SchemaCheckError> {
    if message.field.len() != 2
        || message.field[0].name.as_deref() != Some("key")
        || message.field[0].number != Some(1)
        || message.field[1].name.as_deref() != Some("value")
        || message.field[1].number != Some(2)
        || message
            .field
            .iter()
            .any(|field| field.label != Some(Label::Optional as i32) || field.oneof_index.is_some())
        || !message.extension.is_empty()
        || !message.nested_type.is_empty()
        || !message.enum_type.is_empty()
        || !message.oneof_decl.is_empty()
    {
        return invalid(format!("malformed map entry {qualified}"));
    }
    let key_type = Type::try_from(required_number(
        message.field[0].r#type,
        "map entry key type",
    )?)
    .map_err(|_| {
        SchemaCheckError::InvalidDescriptor(format!("invalid map entry key type in {qualified}"))
    })?;
    if !matches!(
        key_type,
        Type::Int32
            | Type::Int64
            | Type::Uint32
            | Type::Uint64
            | Type::Sint32
            | Type::Sint64
            | Type::Fixed32
            | Type::Fixed64
            | Type::Sfixed32
            | Type::Sfixed64
            | Type::Bool
            | Type::String
    ) {
        return invalid(format!("invalid map entry key type in {qualified}"));
    }
    Ok(())
}

fn validate_enum(
    scope: &str,
    enumeration: &EnumDescriptorProto,
    syntax: &str,
) -> Result<(), SchemaCheckError> {
    let name = required_text(enumeration.name.as_deref(), "enum name")?;
    let qualified = format!("{scope}.{name}");
    if enumeration.value.is_empty() {
        return invalid(format!("enum {qualified} contains no values"));
    }
    let reserved_names = validate_reserved_names(&qualified, "enum", &enumeration.reserved_name)?;
    let reserved_ranges = validate_enum_reserved_ranges(&qualified, &enumeration.reserved_range)?;
    let allow_alias = enumeration
        .options
        .as_ref()
        .and_then(|options| options.allow_alias)
        .unwrap_or(false);
    let mut names = BTreeSet::new();
    let mut numbers = BTreeSet::new();
    for value in &enumeration.value {
        let value_name = required_text(value.name.as_deref(), "enum value name")?;
        let number = required_number(value.number, "enum value number")?;
        if !names.insert(value_name) {
            return invalid(format!(
                "duplicate enum value name {qualified}.{value_name}"
            ));
        }
        if !numbers.insert(number) && !allow_alias {
            return invalid(format!("duplicate enum number {number} in {qualified}"));
        }
        if reserved_names.contains(value_name) {
            return invalid(format!(
                "reserved enum name {qualified}.{value_name} was reused"
            ));
        }
        if range_contains_enum(&reserved_ranges, number) {
            return invalid(format!(
                "reserved enum number {number} in {qualified} was reused"
            ));
        }
    }
    if syntax == "proto3" && enumeration.value[0].number != Some(0) {
        return invalid(format!(
            "first proto3 enum value in {qualified} must be zero"
        ));
    }
    Ok(())
}

fn validate_enum_value_namespace(
    scope: &str,
    enums: &[EnumDescriptorProto],
) -> Result<(), SchemaCheckError> {
    let mut owners = BTreeMap::new();
    for (enum_index, enumeration) in enums.iter().enumerate() {
        for value in &enumeration.value {
            let name = required_text(value.name.as_deref(), "enum value name")?;
            if owners.get(name) == Some(&enum_index) {
                continue;
            }
            if owners.insert(name, enum_index).is_some() {
                return invalid(format!("enum value namespace collision for {scope}.{name}"));
            }
        }
    }
    Ok(())
}

fn validate_services(
    package: &str,
    services: &[ServiceDescriptorProto],
    symbols: &Symbols,
    visible_files: &BTreeSet<String>,
) -> Result<(), SchemaCheckError> {
    let mut names = BTreeSet::new();
    for service in services {
        let name = required_text(service.name.as_deref(), "service name")?;
        if !names.insert(name) {
            return invalid(format!("duplicate service name {package}.{name}"));
        }
        let mut methods = BTreeSet::new();
        for method in &service.method {
            let method_name = required_text(method.name.as_deref(), "method name")?;
            if !methods.insert(method_name) {
                return invalid(format!(
                    "duplicate method name {package}.{name}.{method_name}"
                ));
            }
            for (direction, reference) in [
                ("input", method.input_type.as_deref()),
                ("output", method.output_type.as_deref()),
            ] {
                let reference = required_text(reference, "method type reference")?;
                let Some(origin) = symbols.messages.get(reference) else {
                    return invalid(format!(
                        "unknown type reference {reference} for {direction} of {package}.{name}.{method_name}"
                    ));
                };
                if symbols.map_entries.contains_key(reference) {
                    return invalid(format!(
                        "map entry {reference} cannot be a service method type"
                    ));
                }
                validate_reference_visibility(
                    reference,
                    origin,
                    &format!("{package}.{name}"),
                    method_name,
                    visible_files,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_reserved_names<'a>(
    qualified: &str,
    kind: &str,
    names: &'a [String],
) -> Result<BTreeSet<&'a str>, SchemaCheckError> {
    let mut unique = BTreeSet::new();
    for name in names {
        if name.is_empty() || !unique.insert(name.as_str()) {
            return invalid(format!(
                "duplicate or empty reserved {kind} name in {qualified}"
            ));
        }
    }
    Ok(unique)
}

fn validate_message_reserved_ranges(
    qualified: &str,
    ranges: &[prost_types::descriptor_proto::ReservedRange],
) -> Result<Vec<(i32, i32)>, SchemaCheckError> {
    let mut validated = Vec::new();
    for range in ranges {
        let start = required_number(range.start, "reserved field range start")?;
        let end = required_number(range.end, "reserved field range end")?;
        if start >= end {
            return invalid(format!(
                "invalid reserved field range {start}..{end} in {qualified}"
            ));
        }
        validated.push((start, end));
    }
    ensure_non_overlapping(&mut validated, "reserved field", qualified, false)?;
    Ok(validated)
}

fn validate_enum_reserved_ranges(
    qualified: &str,
    ranges: &[prost_types::enum_descriptor_proto::EnumReservedRange],
) -> Result<Vec<(i32, i32)>, SchemaCheckError> {
    let mut validated = Vec::new();
    for range in ranges {
        let start = required_number(range.start, "reserved enum range start")?;
        let end = required_number(range.end, "reserved enum range end")?;
        if start > end {
            return invalid(format!(
                "invalid reserved enum range {start}..{end} in {qualified}"
            ));
        }
        validated.push((start, end));
    }
    ensure_non_overlapping(&mut validated, "reserved enum", qualified, true)?;
    Ok(validated)
}

fn ensure_non_overlapping(
    ranges: &mut [(i32, i32)],
    kind: &str,
    qualified: &str,
    inclusive_end: bool,
) -> Result<(), SchemaCheckError> {
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| {
        if inclusive_end {
            pair[1].0 <= pair[0].1
        } else {
            pair[1].0 < pair[0].1
        }
    }) {
        return invalid(format!("overlapping {kind} ranges in {qualified}"));
    }
    Ok(())
}

fn range_contains_message(ranges: &[(i32, i32)], number: i32) -> bool {
    ranges
        .iter()
        .any(|(start, end)| (*start..*end).contains(&number))
}

fn range_contains_enum(ranges: &[(i32, i32)], number: i32) -> bool {
    ranges
        .iter()
        .any(|(start, end)| (*start..=*end).contains(&number))
}

fn invalid<T>(message: String) -> Result<T, SchemaCheckError> {
    Err(SchemaCheckError::InvalidDescriptor(message))
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
    compare_enums(baseline_package, &baseline.enum_type, &current.enum_type)?;
    compare_services(baseline_package, &baseline.service, &current.service)
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
        compare_message_reservations(&qualified, baseline_message, current_message)?;
        compare_oneofs(&qualified, baseline_message, current_message)?;
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

fn compare_oneofs(
    message: &str,
    baseline: &DescriptorProto,
    current: &DescriptorProto,
) -> Result<(), SchemaCheckError> {
    for (index, baseline_oneof) in baseline.oneof_decl.iter().enumerate() {
        let baseline_name = required_text(baseline_oneof.name.as_deref(), "baseline oneof name")?;
        let current_name = current
            .oneof_decl
            .get(index)
            .and_then(|oneof| oneof.name.as_deref())
            .ok_or_else(|| {
                SchemaCheckError::Incompatible(format!(
                    "removed oneof identity {message}.{baseline_name} at index {index}"
                ))
            })?;
        if baseline_name != current_name {
            return Err(SchemaCheckError::Incompatible(format!(
                "oneof identity changed for {message} at index {index}: {baseline_name} became {current_name}"
            )));
        }
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

fn compare_message_reservations(
    message: &str,
    baseline: &DescriptorProto,
    current: &DescriptorProto,
) -> Result<(), SchemaCheckError> {
    for name in &baseline.reserved_name {
        if !current.reserved_name.contains(name) {
            return Err(SchemaCheckError::Incompatible(format!(
                "dropped reserved field name {message}.{name}"
            )));
        }
    }
    for range in &baseline.reserved_range {
        let start = required_number(range.start, "baseline reserved field range start")?;
        let end = required_number(range.end, "baseline reserved field range end")?;
        let covered = current.reserved_range.iter().any(|candidate| {
            candidate
                .start
                .zip(candidate.end)
                .is_some_and(|(candidate_start, candidate_end)| {
                    candidate_start <= start && candidate_end >= end
                })
        });
        if !covered {
            return Err(SchemaCheckError::Incompatible(format!(
                "dropped reserved field range {start}..{end} in {message}"
            )));
        }
    }
    for field in &current.field {
        let name = required_text(field.name.as_deref(), "current field name")?;
        let number = required_number(field.number, "current field number")?;
        if baseline.reserved_name.contains(&name.to_owned()) {
            return Err(SchemaCheckError::Incompatible(format!(
                "reserved field name {message}.{name} was reused"
            )));
        }
        if baseline.reserved_range.iter().any(|range| {
            range
                .start
                .zip(range.end)
                .is_some_and(|(start, end)| (start..end).contains(&number))
        }) {
            return Err(SchemaCheckError::Incompatible(format!(
                "reserved field number {number} in {message} was reused"
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
        compare_enum_reservations(&qualified, baseline_enum, current_enum)?;
        compare_enum_values(&qualified, baseline_enum, current_enum)?;
    }
    Ok(())
}

fn compare_enum_reservations(
    enumeration: &str,
    baseline: &EnumDescriptorProto,
    current: &EnumDescriptorProto,
) -> Result<(), SchemaCheckError> {
    for name in &baseline.reserved_name {
        if !current.reserved_name.contains(name) {
            return Err(SchemaCheckError::Incompatible(format!(
                "dropped reserved enum name {enumeration}.{name}"
            )));
        }
    }
    for range in &baseline.reserved_range {
        let start = required_number(range.start, "baseline reserved enum range start")?;
        let end = required_number(range.end, "baseline reserved enum range end")?;
        let covered = current.reserved_range.iter().any(|candidate| {
            candidate
                .start
                .zip(candidate.end)
                .is_some_and(|(candidate_start, candidate_end)| {
                    candidate_start <= start && candidate_end >= end
                })
        });
        if !covered {
            return Err(SchemaCheckError::Incompatible(format!(
                "dropped reserved enum range {start}..{end} in {enumeration}"
            )));
        }
    }
    for value in &current.value {
        let name = required_text(value.name.as_deref(), "current enum value name")?;
        let number = required_number(value.number, "current enum value number")?;
        if baseline.reserved_name.contains(&name.to_owned()) {
            return Err(SchemaCheckError::Incompatible(format!(
                "reserved enum name {enumeration}.{name} was reused"
            )));
        }
        if baseline.reserved_range.iter().any(|range| {
            range
                .start
                .zip(range.end)
                .is_some_and(|(start, end)| (start..=end).contains(&number))
        }) {
            return Err(SchemaCheckError::Incompatible(format!(
                "reserved enum number {number} in {enumeration} was reused"
            )));
        }
    }
    Ok(())
}

fn compare_services(
    package: &str,
    baseline: &[ServiceDescriptorProto],
    current: &[ServiceDescriptorProto],
) -> Result<(), SchemaCheckError> {
    let current_by_name = current
        .iter()
        .map(|service| {
            required_text(service.name.as_deref(), "service name").map(|name| (name, service))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    for baseline_service in baseline {
        let name = required_text(baseline_service.name.as_deref(), "baseline service name")?;
        let current_service = current_by_name.get(name).copied().ok_or_else(|| {
            SchemaCheckError::Incompatible(format!("removed service {package}.{name}"))
        })?;
        let current_methods = current_service
            .method
            .iter()
            .map(|method| {
                required_text(method.name.as_deref(), "method name").map(|name| (name, method))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        for baseline_method in &baseline_service.method {
            let method_name =
                required_text(baseline_method.name.as_deref(), "baseline method name")?;
            let current_method = current_methods.get(method_name).copied().ok_or_else(|| {
                SchemaCheckError::Incompatible(format!(
                    "removed method {package}.{name}.{method_name}"
                ))
            })?;
            if baseline_method.input_type != current_method.input_type
                || baseline_method.output_type != current_method.output_type
                || baseline_method.client_streaming != current_method.client_streaming
                || baseline_method.server_streaming != current_method.server_streaming
            {
                return Err(SchemaCheckError::Incompatible(format!(
                    "incompatible method signature for {package}.{name}.{method_name}"
                )));
            }
        }
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
