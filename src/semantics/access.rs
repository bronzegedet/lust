use crate::ast::Expr;
use crate::dispatch::infer_expr_type;
use crate::typecheck::{InferredType, TypeInfo};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub enum LoweredAssignmentTarget {
    Ident {
        name: String,
        ty: InferredType,
        is_global: bool,
    },
    TypedListIndex {
        target: Expr,
        index: Expr,
        item_type: InferredType,
    },
    DynamicIndex {
        target: Expr,
        index: Expr,
    },
    TypedStructField {
        target: Expr,
        struct_name: String,
        field: String,
        field_type: InferredType,
    },
    DynamicMember {
        target: Expr,
        field: String,
    },
    UnsupportedSlice,
    Unsupported,
}

#[derive(Debug, Clone)]
pub enum LoweredReadAccess {
    TypedListIndex {
        target: Expr,
        index: Expr,
        item_type: InferredType,
    },
    StringIndex {
        target: Expr,
        index: Expr,
    },
    TypedListSlice {
        target: Expr,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        item_type: InferredType,
    },
    StringSlice {
        target: Expr,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
    },
    DynamicIndex {
        target: Expr,
        index: Expr,
    },
    TypedStructField {
        target: Expr,
        struct_name: String,
        field: String,
        field_type: InferredType,
    },
    EnumVariantValue {
        enum_name: String,
        variant: String,
    },
    BuiltinLength {
        target: Expr,
    },
    DynamicMember {
        target: Expr,
        field: String,
    },
    DynamicSlice {
        target: Expr,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
    },
}

pub fn lower_assignment_target(
    expr: &Expr,
    local_types: &HashMap<String, InferredType>,
    type_info: &TypeInfo,
    globals: &HashSet<String>,
) -> LoweredAssignmentTarget {
    match expr {
        Expr::Ident(name) => LoweredAssignmentTarget::Ident {
            name: name.clone(),
            ty: if globals.contains(name) {
                InferredType::Dynamic
            } else {
                local_types.get(name).cloned().unwrap_or(InferredType::Dynamic)
            },
            is_global: globals.contains(name),
        },
        Expr::Index(target, index) => match infer_expr_type(target, local_types, type_info, globals) {
            InferredType::List(inner) => LoweredAssignmentTarget::TypedListIndex {
                target: (**target).clone(),
                index: (**index).clone(),
                item_type: (*inner).clone(),
            },
            _ => LoweredAssignmentTarget::DynamicIndex {
                target: (**target).clone(),
                index: (**index).clone(),
            },
        },
        Expr::Member(target, field) => match infer_expr_type(target, local_types, type_info, globals) {
            InferredType::Struct(struct_name) => {
                let field_type = type_info
                    .struct_fields
                    .get(&struct_name)
                    .and_then(|fields| fields.get(field))
                    .cloned()
                    .unwrap_or(InferredType::Dynamic);
                LoweredAssignmentTarget::TypedStructField {
                    target: (**target).clone(),
                    struct_name,
                    field: field.clone(),
                    field_type,
                }
            }
            _ => LoweredAssignmentTarget::DynamicMember {
                target: (**target).clone(),
                field: field.clone(),
            },
        },
        Expr::Slice(_, _, _) => LoweredAssignmentTarget::UnsupportedSlice,
        _ => LoweredAssignmentTarget::Unsupported,
    }
}

pub fn lower_read_access(
    expr: &Expr,
    local_types: &HashMap<String, InferredType>,
    type_info: &TypeInfo,
    globals: &HashSet<String>,
) -> Option<LoweredReadAccess> {
    match expr {
        Expr::Index(target, index) => match infer_expr_type(target, local_types, type_info, globals) {
            InferredType::List(inner) => Some(LoweredReadAccess::TypedListIndex {
                target: (**target).clone(),
                index: (**index).clone(),
                item_type: (*inner).clone(),
            }),
            InferredType::String => Some(LoweredReadAccess::StringIndex {
                target: (**target).clone(),
                index: (**index).clone(),
            }),
            _ => Some(LoweredReadAccess::DynamicIndex {
                target: (**target).clone(),
                index: (**index).clone(),
            }),
        },
        Expr::Slice(target, start, end) => match infer_expr_type(target, local_types, type_info, globals) {
            InferredType::List(inner) => Some(LoweredReadAccess::TypedListSlice {
                target: (**target).clone(),
                start: start.clone(),
                end: end.clone(),
                item_type: (*inner).clone(),
            }),
            InferredType::String => Some(LoweredReadAccess::StringSlice {
                target: (**target).clone(),
                start: start.clone(),
                end: end.clone(),
            }),
            _ => Some(LoweredReadAccess::DynamicSlice {
                target: (**target).clone(),
                start: start.clone(),
                end: end.clone(),
            }),
        },
        Expr::Member(target, field) => {
            if let Expr::Ident(enum_name) = target.as_ref() {
                if type_info
                    .variant_to_enum
                    .get(field)
                    .map(|owner| owner == enum_name)
                    .unwrap_or(false)
                {
                    return Some(LoweredReadAccess::EnumVariantValue {
                        enum_name: enum_name.clone(),
                        variant: field.clone(),
                    });
                }
            }
            if field == "length" {
                return Some(LoweredReadAccess::BuiltinLength {
                    target: (**target).clone(),
                });
            }
            match infer_expr_type(target, local_types, type_info, globals) {
                InferredType::Struct(struct_name) => {
                    let field_type = type_info
                        .struct_fields
                        .get(&struct_name)
                        .and_then(|fields| fields.get(field))
                        .cloned()
                        .unwrap_or(InferredType::Dynamic);
                    Some(LoweredReadAccess::TypedStructField {
                        target: (**target).clone(),
                        struct_name,
                        field: field.clone(),
                        field_type,
                    })
                }
                _ => Some(LoweredReadAccess::DynamicMember {
                    target: (**target).clone(),
                    field: field.clone(),
                }),
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowers_global_identifier_assignment_target() {
        let mut globals = HashSet::new();
        globals.insert("count".to_string());
        let lowered = lower_assignment_target(
            &Expr::Ident("count".to_string()),
            &HashMap::new(),
            &TypeInfo::default(),
            &globals,
        );
        assert!(matches!(
            lowered,
            LoweredAssignmentTarget::Ident {
                ref name,
                ty: InferredType::Dynamic,
                is_global: true
            } if name == "count"
        ));
    }

    #[test]
    fn lowers_typed_struct_field_assignment_target() {
        let mut local_types = HashMap::new();
        local_types.insert("user".to_string(), InferredType::Struct("User".to_string()));

        let mut type_info = TypeInfo::default();
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), InferredType::String);
        type_info.struct_fields.insert("User".to_string(), fields);

        let lowered = lower_assignment_target(
            &Expr::Member(Box::new(Expr::Ident("user".to_string())), "name".to_string()),
            &local_types,
            &type_info,
            &HashSet::new(),
        );
        assert!(matches!(
            lowered,
            LoweredAssignmentTarget::TypedStructField {
                ref struct_name,
                ref field,
                field_type: InferredType::String,
                ..
            } if struct_name == "User" && field == "name"
        ));
    }

    #[test]
    fn lowers_typed_list_index_assignment_target() {
        let mut local_types = HashMap::new();
        local_types.insert(
            "items".to_string(),
            InferredType::List(Box::new(InferredType::Number)),
        );

        let lowered = lower_assignment_target(
            &Expr::Index(
                Box::new(Expr::Ident("items".to_string())),
                Box::new(Expr::Number(0.0)),
            ),
            &local_types,
            &TypeInfo::default(),
            &HashSet::new(),
        );
        assert!(matches!(
            lowered,
            LoweredAssignmentTarget::TypedListIndex {
                item_type: InferredType::Number,
                ..
            }
        ));
    }

    #[test]
    fn lowers_string_index_read_access() {
        let mut local_types = HashMap::new();
        local_types.insert("text".to_string(), InferredType::String);
        let lowered = lower_read_access(
            &Expr::Index(
                Box::new(Expr::Ident("text".to_string())),
                Box::new(Expr::Number(0.0)),
            ),
            &local_types,
            &TypeInfo::default(),
            &HashSet::new(),
        )
        .expect("expected lowered read access");
        assert!(matches!(lowered, LoweredReadAccess::StringIndex { .. }));
    }

    #[test]
    fn lowers_enum_variant_member_read_access() {
        let mut type_info = TypeInfo::default();
        type_info
            .variant_to_enum
            .insert("Up".to_string(), "Key".to_string());
        let lowered = lower_read_access(
            &Expr::Member(Box::new(Expr::Ident("Key".to_string())), "Up".to_string()),
            &HashMap::new(),
            &type_info,
            &HashSet::new(),
        )
        .expect("expected lowered read access");
        assert!(matches!(
            lowered,
            LoweredReadAccess::EnumVariantValue {
                ref enum_name,
                ref variant
            } if enum_name == "Key" && variant == "Up"
        ));
    }
}
