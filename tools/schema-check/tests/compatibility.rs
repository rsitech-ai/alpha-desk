use prost_types::{
    DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto,
    FileDescriptorProto, FileDescriptorSet, MessageOptions, MethodDescriptorProto,
    OneofDescriptorProto, ServiceDescriptorProto,
    descriptor_proto::ReservedRange,
    enum_descriptor_proto::EnumReservedRange,
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

fn current_only_file() -> FileDescriptorProto {
    FileDescriptorProto {
        name: Some("extra/v1/current.proto".to_owned()),
        package: Some("hl.extra.v1".to_owned()),
        syntax: Some("proto3".to_owned()),
        message_type: vec![DescriptorProto {
            name: Some("Added".to_owned()),
            field: vec![field("value", 1, Type::String, Label::Optional)],
            ..DescriptorProto::default()
        }],
        ..FileDescriptorProto::default()
    }
}

#[test]
fn current_only_duplicate_field_names_and_numbers_are_rejected() {
    let duplicate_number = error_for(|current| {
        let mut file = current_only_file();
        file.message_type[0]
            .field
            .push(field("other", 1, Type::String, Label::Optional));
        current.file.push(file);
    });
    assert!(duplicate_number.contains("duplicate field number"));

    let duplicate_name = error_for(|current| {
        let mut file = current_only_file();
        file.message_type[0]
            .field
            .push(field("value", 2, Type::String, Label::Optional));
        current.file.push(file);
    });
    assert!(duplicate_name.contains("duplicate field name"));
}

#[test]
fn current_only_nested_and_enum_duplicates_are_rejected() {
    let nested = error_for(|current| {
        let mut file = current_only_file();
        file.message_type[0].nested_type = vec![
            DescriptorProto {
                name: Some("Nested".to_owned()),
                ..DescriptorProto::default()
            },
            DescriptorProto {
                name: Some("Nested".to_owned()),
                ..DescriptorProto::default()
            },
        ];
        current.file.push(file);
    });
    assert!(nested.contains("duplicate nested type name"));

    let enumeration = error_for(|current| {
        let mut file = current_only_file();
        file.enum_type = vec![EnumDescriptorProto {
            name: Some("State".to_owned()),
            value: vec![
                EnumValueDescriptorProto {
                    name: Some("READY".to_owned()),
                    number: Some(0),
                    ..EnumValueDescriptorProto::default()
                },
                EnumValueDescriptorProto {
                    name: Some("READY".to_owned()),
                    number: Some(1),
                    ..EnumValueDescriptorProto::default()
                },
            ],
            ..EnumDescriptorProto::default()
        }];
        current.file.push(file);
    });
    assert!(enumeration.contains("duplicate enum value name"));
}

#[test]
fn current_only_duplicate_enum_numbers_require_explicit_aliasing() {
    let no_alias = error_for(|current| {
        let mut file = current_only_file();
        file.enum_type = vec![EnumDescriptorProto {
            name: Some("State".to_owned()),
            value: vec![
                EnumValueDescriptorProto {
                    name: Some("UNKNOWN".to_owned()),
                    number: Some(0),
                    ..EnumValueDescriptorProto::default()
                },
                EnumValueDescriptorProto {
                    name: Some("ALSO_UNKNOWN".to_owned()),
                    number: Some(0),
                    ..EnumValueDescriptorProto::default()
                },
            ],
            ..EnumDescriptorProto::default()
        }];
        current.file.push(file);
    });
    assert!(no_alias.contains("duplicate enum number"));

    let baseline = descriptor();
    let mut aliased = baseline.clone();
    let mut file = current_only_file();
    file.enum_type = vec![EnumDescriptorProto {
        name: Some("State".to_owned()),
        value: vec![
            EnumValueDescriptorProto {
                name: Some("UNKNOWN".to_owned()),
                number: Some(0),
                ..EnumValueDescriptorProto::default()
            },
            EnumValueDescriptorProto {
                name: Some("ALSO_UNKNOWN".to_owned()),
                number: Some(0),
                ..EnumValueDescriptorProto::default()
            },
        ],
        options: Some(prost_types::EnumOptions {
            allow_alias: Some(true),
            ..prost_types::EnumOptions::default()
        }),
        ..EnumDescriptorProto::default()
    }];
    aliased.file.push(file);
    check_file_descriptor_sets(&baseline, &aliased).unwrap();
}

#[test]
fn baseline_reserved_field_names_and_ranges_cannot_be_dropped_or_reused() {
    let mut baseline = descriptor();
    baseline.file[0].message_type[0].reserved_name = vec!["legacy".to_owned()];
    baseline.file[0].message_type[0].reserved_range = vec![ReservedRange {
        start: Some(10),
        end: Some(12),
    }];

    let mut dropped = baseline.clone();
    dropped.file[0].message_type[0].reserved_name.clear();
    assert!(
        check_file_descriptor_sets(&baseline, &dropped)
            .unwrap_err()
            .to_string()
            .contains("dropped reserved field name")
    );

    let mut reused_name = baseline.clone();
    reused_name.file[0].message_type[0].field.push(field(
        "legacy",
        20,
        Type::String,
        Label::Optional,
    ));
    assert!(
        check_file_descriptor_sets(&baseline, &reused_name)
            .unwrap_err()
            .to_string()
            .contains("reserved field name")
    );

    let mut reused_number = baseline.clone();
    reused_number.file[0].message_type[0].field.push(field(
        "new_value",
        10,
        Type::String,
        Label::Optional,
    ));
    assert!(
        check_file_descriptor_sets(&baseline, &reused_number)
            .unwrap_err()
            .to_string()
            .contains("reserved field number")
    );
}

#[test]
fn baseline_reserved_enum_names_and_ranges_cannot_be_dropped_or_reused() {
    let mut baseline = descriptor();
    baseline.file[0].enum_type[0].reserved_name = vec!["LEGACY".to_owned()];
    baseline.file[0].enum_type[0].reserved_range = vec![EnumReservedRange {
        start: Some(10),
        end: Some(11),
    }];

    let mut dropped = baseline.clone();
    dropped.file[0].enum_type[0].reserved_range.clear();
    assert!(
        check_file_descriptor_sets(&baseline, &dropped)
            .unwrap_err()
            .to_string()
            .contains("dropped reserved enum range")
    );

    let mut reused_name = baseline.clone();
    reused_name.file[0].enum_type[0]
        .value
        .push(EnumValueDescriptorProto {
            name: Some("LEGACY".to_owned()),
            number: Some(20),
            ..EnumValueDescriptorProto::default()
        });
    assert!(
        check_file_descriptor_sets(&baseline, &reused_name)
            .unwrap_err()
            .to_string()
            .contains("reserved enum name")
    );

    let mut reused_number = baseline.clone();
    reused_number.file[0].enum_type[0]
        .value
        .push(EnumValueDescriptorProto {
            name: Some("NEW_VALUE".to_owned()),
            number: Some(10),
            ..EnumValueDescriptorProto::default()
        });
    assert!(
        check_file_descriptor_sets(&baseline, &reused_number)
            .unwrap_err()
            .to_string()
            .contains("reserved enum number")
    );
}

#[test]
fn invalid_oneof_indices_and_malformed_map_entries_are_rejected() {
    let oneof = error_for(|current| {
        let mut file = current_only_file();
        file.message_type[0].oneof_decl = vec![OneofDescriptorProto {
            name: Some("choice".to_owned()),
            ..OneofDescriptorProto::default()
        }];
        file.message_type[0].field[0].oneof_index = Some(3);
        current.file.push(file);
    });
    assert!(oneof.contains("invalid oneof index"));

    let map = error_for(|current| {
        let mut file = current_only_file();
        file.message_type[0].nested_type = vec![DescriptorProto {
            name: Some("ItemsEntry".to_owned()),
            field: vec![field("key", 1, Type::String, Label::Optional)],
            options: Some(MessageOptions {
                map_entry: Some(true),
                ..MessageOptions::default()
            }),
            ..DescriptorProto::default()
        }];
        current.file.push(file);
    });
    assert!(map.contains("map entry"));
}

#[test]
fn duplicate_services_methods_and_invalid_type_references_are_rejected() {
    let services = error_for(|current| {
        let mut file = current_only_file();
        let service = ServiceDescriptorProto {
            name: Some("Reader".to_owned()),
            method: vec![
                MethodDescriptorProto {
                    name: Some("Get".to_owned()),
                    input_type: Some(".hl.extra.v1.Added".to_owned()),
                    output_type: Some(".hl.extra.v1.Added".to_owned()),
                    ..MethodDescriptorProto::default()
                },
                MethodDescriptorProto {
                    name: Some("Get".to_owned()),
                    input_type: Some(".hl.extra.v1.Added".to_owned()),
                    output_type: Some(".hl.extra.v1.Added".to_owned()),
                    ..MethodDescriptorProto::default()
                },
            ],
            ..ServiceDescriptorProto::default()
        };
        file.service = vec![service.clone(), service];
        current.file.push(file);
    });
    assert!(
        services.contains("duplicate service name") || services.contains("duplicate method name")
    );

    let type_reference = error_for(|current| {
        let mut file = current_only_file();
        file.message_type[0].field[0].r#type = Some(Type::Message as i32);
        file.message_type[0].field[0].type_name = Some(".hl.extra.v1.DoesNotExist".to_owned());
        current.file.push(file);
    });
    assert!(type_reference.contains("unknown type reference"));
}

#[test]
fn cross_file_references_require_a_visible_declared_import() {
    let missing_import = error_for(|current| {
        let file = current_only_file();
        current.file[0].message_type[0].field.push(message_field(
            "added",
            3,
            ".hl.extra.v1.Added",
            Label::Optional,
        ));
        current.file.push(file);
    });
    assert!(missing_import.contains("missing import"));

    let baseline = descriptor();
    let mut current = baseline.clone();
    let file = current_only_file();
    current.file[0]
        .dependency
        .push("extra/v1/current.proto".to_owned());
    current.file[0].message_type[0].field.push(message_field(
        "added",
        3,
        ".hl.extra.v1.Added",
        Label::Optional,
    ));
    current.file.push(file);
    check_file_descriptor_sets(&baseline, &current).unwrap();

    let mut through_public_import = baseline.clone();
    let provider = FileDescriptorProto {
        name: Some("provider/v1/types.proto".to_owned()),
        package: Some("hl.provider.v1".to_owned()),
        syntax: Some("proto3".to_owned()),
        message_type: vec![DescriptorProto {
            name: Some("ProviderValue".to_owned()),
            field: vec![field("value", 1, Type::String, Label::Optional)],
            ..DescriptorProto::default()
        }],
        ..FileDescriptorProto::default()
    };
    let reexport = FileDescriptorProto {
        name: Some("reexport/v1/types.proto".to_owned()),
        package: Some("hl.reexport.v1".to_owned()),
        syntax: Some("proto3".to_owned()),
        dependency: vec!["provider/v1/types.proto".to_owned()],
        public_dependency: vec![0],
        ..FileDescriptorProto::default()
    };
    through_public_import.file[0]
        .dependency
        .push("reexport/v1/types.proto".to_owned());
    through_public_import.file[0].message_type[0]
        .field
        .push(message_field(
            "provider",
            3,
            ".hl.provider.v1.ProviderValue",
            Label::Optional,
        ));
    through_public_import.file.extend([provider, reexport]);
    check_file_descriptor_sets(&baseline, &through_public_import).unwrap();
}

#[test]
fn message_declarations_share_one_namespace() {
    let collision = error_for(|current| {
        let mut file = current_only_file();
        file.message_type[0].oneof_decl = vec![OneofDescriptorProto {
            name: Some("value".to_owned()),
            ..OneofDescriptorProto::default()
        }];
        current.file.push(file);
    });
    assert!(collision.contains("message namespace collision"));

    let baseline = descriptor();
    let mut current = baseline.clone();
    let mut file = current_only_file();
    file.message_type[0].oneof_decl = vec![OneofDescriptorProto {
        name: Some("choice".to_owned()),
        ..OneofDescriptorProto::default()
    }];
    current.file.push(file);
    check_file_descriptor_sets(&baseline, &current).unwrap();
}

#[test]
fn sibling_enum_values_share_the_enclosing_scope() {
    let collision = error_for(|current| {
        let mut file = current_only_file();
        file.enum_type = vec![
            enumeration("FirstState", "UNKNOWN"),
            enumeration("SecondState", "UNKNOWN"),
        ];
        current.file.push(file);
    });
    assert!(collision.contains("enum value namespace collision"));

    let cross_file_collision = error_for(|current| {
        let mut first = current_only_file();
        first.enum_type = vec![enumeration("FirstState", "UNKNOWN")];
        let mut second = current_only_file();
        second.name = Some("extra/v1/second.proto".to_owned());
        second.message_type[0].name = Some("SecondAdded".to_owned());
        second.enum_type = vec![enumeration("SecondState", "UNKNOWN")];
        current.file.extend([first, second]);
    });
    assert!(cross_file_collision.contains("enum value namespace collision"));

    let baseline = descriptor();
    let mut current = baseline.clone();
    let mut file = current_only_file();
    file.enum_type = vec![
        enumeration("FirstState", "FIRST_UNKNOWN"),
        enumeration("SecondState", "SECOND_UNKNOWN"),
    ];
    current.file.push(file);
    check_file_descriptor_sets(&baseline, &current).unwrap();
}

#[test]
fn map_entries_require_exactly_one_legal_parent_map_field() {
    let unlinked = error_for(|current| {
        let mut file = current_only_file();
        file.message_type[0]
            .nested_type
            .push(string_map_entry("ItemsEntry"));
        current.file.push(file);
    });
    assert!(unlinked.contains("map entry"));
    assert!(unlinked.contains("exactly one"));

    let optional = error_for(|current| {
        let mut file = current_only_file();
        file.message_type[0]
            .nested_type
            .push(string_map_entry("ItemsEntry"));
        file.message_type[0].field.push(message_field(
            "items",
            2,
            ".hl.extra.v1.Added.ItemsEntry",
            Label::Optional,
        ));
        current.file.push(file);
    });
    assert!(optional.contains("legal repeated map field"));

    let reused = error_for(|current| {
        let mut file = current_only_file();
        file.message_type[0]
            .nested_type
            .push(string_map_entry("ItemsEntry"));
        file.message_type[0].field.extend([
            message_field("items", 2, ".hl.extra.v1.Added.ItemsEntry", Label::Repeated),
            message_field(
                "other_items",
                3,
                ".hl.extra.v1.Added.ItemsEntry",
                Label::Repeated,
            ),
        ]);
        current.file.push(file);
    });
    assert!(reused.contains("exactly one"));

    let baseline = descriptor();
    let mut current = baseline.clone();
    let mut file = current_only_file();
    file.message_type[0]
        .nested_type
        .push(string_map_entry("ItemsEntry"));
    file.message_type[0].field.push(message_field(
        "items",
        2,
        ".hl.extra.v1.Added.ItemsEntry",
        Label::Repeated,
    ));
    current.file.push(file);
    check_file_descriptor_sets(&baseline, &current).unwrap();
}

#[test]
fn synthetic_optional_oneofs_and_baseline_oneof_identity_are_enforced() {
    let malformed_optional = error_for(|current| {
        let mut file = current_only_file();
        file.message_type[0].oneof_decl = vec![OneofDescriptorProto {
            name: Some("_value".to_owned()),
            ..OneofDescriptorProto::default()
        }];
        file.message_type[0].field[0].oneof_index = Some(0);
        file.message_type[0].field[0].proto3_optional = Some(true);
        file.message_type[0]
            .field
            .push(field("also_value", 2, Type::String, Label::Optional));
        file.message_type[0].field[1].oneof_index = Some(0);
        current.file.push(file);
    });
    assert!(malformed_optional.contains("synthetic proto3 optional oneof"));

    let mut baseline = descriptor();
    baseline.file[0].message_type[0].oneof_decl = vec![OneofDescriptorProto {
        name: Some("identity".to_owned()),
        ..OneofDescriptorProto::default()
    }];
    baseline.file[0].message_type[0].field[0].oneof_index = Some(0);
    let mut current = baseline.clone();
    current.file[0].message_type[0].oneof_decl[0].name = Some("renamed".to_owned());
    assert!(
        check_file_descriptor_sets(&baseline, &current)
            .unwrap_err()
            .to_string()
            .contains("oneof identity")
    );
}

fn message_field(name: &str, number: i32, type_name: &str, label: Label) -> FieldDescriptorProto {
    let mut field = field(name, number, Type::Message, label);
    field.type_name = Some(type_name.to_owned());
    field
}

fn enumeration(name: &str, zero_name: &str) -> EnumDescriptorProto {
    EnumDescriptorProto {
        name: Some(name.to_owned()),
        value: vec![EnumValueDescriptorProto {
            name: Some(zero_name.to_owned()),
            number: Some(0),
            ..EnumValueDescriptorProto::default()
        }],
        ..EnumDescriptorProto::default()
    }
}

fn string_map_entry(name: &str) -> DescriptorProto {
    DescriptorProto {
        name: Some(name.to_owned()),
        field: vec![
            field("key", 1, Type::String, Label::Optional),
            field("value", 2, Type::String, Label::Optional),
        ],
        options: Some(MessageOptions {
            map_entry: Some(true),
            ..MessageOptions::default()
        }),
        ..DescriptorProto::default()
    }
}
