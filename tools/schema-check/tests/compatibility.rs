use prost_types::{
    DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto,
    FileDescriptorProto, FileDescriptorSet,
    field_descriptor_proto::{Label, Type},
};
use schema_check::{MAX_DESCRIPTOR_BYTES, check_file_descriptor_sets, decode_descriptor_set};

fn field(name: &str, number: i32, field_type: Type, label: Label) -> FieldDescriptorProto {
    FieldDescriptorProto {
        name: Some(name.to_owned()),
        number: Some(number),
        label: Some(label as i32),
        r#type: Some(field_type as i32),
        ..FieldDescriptorProto::default()
    }
}

fn descriptor() -> FileDescriptorSet {
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("canonical/v1/test.proto".to_owned()),
            package: Some("hl.canonical.v1".to_owned()),
            syntax: Some("proto3".to_owned()),
            message_type: vec![DescriptorProto {
                name: Some("Envelope".to_owned()),
                field: vec![
                    field("event_id", 1, Type::String, Label::Optional),
                    field("market_ids", 2, Type::String, Label::Repeated),
                ],
                ..DescriptorProto::default()
            }],
            enum_type: vec![EnumDescriptorProto {
                name: Some("Confirmation".to_owned()),
                value: vec![
                    EnumValueDescriptorProto {
                        name: Some("UNSPECIFIED".to_owned()),
                        number: Some(0),
                        ..EnumValueDescriptorProto::default()
                    },
                    EnumValueDescriptorProto {
                        name: Some("COMMITTED".to_owned()),
                        number: Some(1),
                        ..EnumValueDescriptorProto::default()
                    },
                ],
                ..EnumDescriptorProto::default()
            }],
            ..FileDescriptorProto::default()
        }],
    }
}

fn error_for(mutator: impl FnOnce(&mut FileDescriptorSet)) -> String {
    let baseline = descriptor();
    let mut current = baseline.clone();
    mutator(&mut current);
    check_file_descriptor_sets(&baseline, &current)
        .expect_err("the mutation must be incompatible")
        .to_string()
}

#[test]
fn identical_descriptors_are_compatible() {
    let descriptor = descriptor();
    check_file_descriptor_sets(&descriptor, &descriptor).unwrap();
}

#[test]
fn removed_field_is_rejected() {
    let error = error_for(|current| {
        current.file[0].message_type[0].field.remove(0);
    });
    assert!(error.contains("removed field"));
    assert!(error.contains("event_id"));
}

#[test]
fn field_number_reuse_or_name_replacement_is_rejected() {
    let error = error_for(|current| {
        current.file[0].message_type[0].field[0].name = Some("replacement".to_owned());
    });
    assert!(error.contains("field number 1"));
    assert!(error.contains("event_id"));
    assert!(error.contains("replacement"));
}

#[test]
fn field_renumbering_is_rejected() {
    let error = error_for(|current| {
        current.file[0].message_type[0].field[0].number = Some(7);
    });
    assert!(error.contains("renumbered field"));
}

#[test]
fn incompatible_wire_type_and_cardinality_are_rejected() {
    let type_error = error_for(|current| {
        current.file[0].message_type[0].field[0].r#type = Some(Type::Uint64 as i32);
    });
    assert!(type_error.contains("wire type"));

    let cardinality_error = error_for(|current| {
        current.file[0].message_type[0].field[1].label = Some(Label::Optional as i32);
    });
    assert!(cardinality_error.contains("cardinality"));
}

#[test]
fn removed_and_renumbered_enum_values_are_rejected() {
    let removed = error_for(|current| {
        current.file[0].enum_type[0].value.pop();
    });
    assert!(removed.contains("removed enum value"));

    let renumbered = error_for(|current| {
        current.file[0].enum_type[0].value[1].number = Some(9);
    });
    assert!(renumbered.contains("renumbered enum value"));
}

#[test]
fn enum_number_reuse_is_rejected() {
    let error = error_for(|current| {
        current.file[0].enum_type[0].value[1].name = Some("REPLACEMENT".to_owned());
    });
    assert!(error.contains("enum number 1"));
    assert!(error.contains("COMMITTED"));
    assert!(error.contains("REPLACEMENT"));
}

#[test]
fn package_or_semantic_major_path_drift_is_rejected() {
    let package = error_for(|current| {
        current.file[0].package = Some("hl.canonical.v2".to_owned());
    });
    assert!(package.contains("semantic-major/package drift"));

    let path = error_for(|current| {
        current.file[0].name = Some("canonical/v2/test.proto".to_owned());
    });
    assert!(path.contains("semantic-major/package drift"));
}

#[test]
fn malformed_and_oversized_descriptor_inputs_are_actionable() {
    let malformed = decode_descriptor_set("baseline", &[0xff, 0xff])
        .expect_err("malformed protobuf must fail")
        .to_string();
    assert!(malformed.contains("baseline"));
    assert!(malformed.contains("decode"));

    let oversized = vec![0_u8; MAX_DESCRIPTOR_BYTES + 1];
    let error = decode_descriptor_set("current", &oversized)
        .expect_err("oversized input must fail before decode")
        .to_string();
    assert!(error.contains("current"));
    assert!(error.contains("exceeds"));
    assert!(error.contains(&MAX_DESCRIPTOR_BYTES.to_string()));
}
