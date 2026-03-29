use crate::ast::Pattern;
use crate::typecheck::{InferredType, TypeInfo};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternAccessStep {
    StructField(String),
    EnumField(String, usize),
    ListIndex(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternBinding {
    pub name: String,
    pub path: Vec<PatternAccessStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternLengthCheckOp {
    Exact,
    AtLeast,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoweredPatternCheckKind {
    AlwaysFalse,
    Number(f64),
    StringLit(String),
    Bool(bool),
    Null,
    ListLength {
        len: usize,
        op: PatternLengthCheckOp,
    },
    StructType(String),
    EnumVariant {
        enum_name: Option<String>,
        variant: String,
        arity: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoweredPatternCheck {
    pub path: Vec<PatternAccessStep>,
    pub input_ty: InferredType,
    pub kind: LoweredPatternCheckKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoweredPatternBinding {
    pub name: String,
    pub path: Vec<PatternAccessStep>,
    pub ty: InferredType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoweredDestructure {
    pub target_ty: InferredType,
    pub checks: Vec<LoweredPatternCheck>,
    pub bindings: Vec<LoweredPatternBinding>,
}

pub fn collect_pattern_binding_names(pattern: &Pattern, names: &mut HashSet<String>) {
    match pattern {
        Pattern::Bind(name) => {
            names.insert(name.clone());
        }
        Pattern::List(parts, _) | Pattern::EnumVariant(_, parts) => {
            for pat in parts {
                collect_pattern_binding_names(pat, names);
            }
        }
        Pattern::Struct(_, fields) => {
            for (_, pat) in fields {
                collect_pattern_binding_names(pat, names);
            }
        }
        Pattern::Wildcard
        | Pattern::Number(_)
        | Pattern::StringLit(_)
        | Pattern::Bool(_)
        | Pattern::Null => {}
    }
}

pub fn collect_pattern_binding_paths(pattern: &Pattern) -> Vec<PatternBinding> {
    let mut bindings = Vec::new();
    collect_pattern_binding_paths_at(pattern, &[], &mut bindings);
    bindings
}

fn collect_pattern_binding_paths_at(
    pattern: &Pattern,
    path: &[PatternAccessStep],
    bindings: &mut Vec<PatternBinding>,
) {
    match pattern {
        Pattern::Bind(name) => bindings.push(PatternBinding {
            name: name.clone(),
            path: path.to_vec(),
        }),
        Pattern::List(parts, _) | Pattern::EnumVariant(_, parts) => {
            for (idx, pat) in parts.iter().enumerate() {
                let mut child_path = path.to_vec();
                child_path.push(match pattern {
                    Pattern::List(_, _) => PatternAccessStep::ListIndex(idx),
                    Pattern::EnumVariant(name, _) => PatternAccessStep::EnumField(name.clone(), idx),
                    _ => unreachable!(),
                });
                collect_pattern_binding_paths_at(pat, &child_path, bindings);
            }
        }
        Pattern::Struct(_, fields) => {
            for (field, pat) in fields {
                let mut child_path = path.to_vec();
                child_path.push(PatternAccessStep::StructField(field.clone()));
                collect_pattern_binding_paths_at(pat, &child_path, bindings);
            }
        }
        Pattern::Wildcard
        | Pattern::Number(_)
        | Pattern::StringLit(_)
        | Pattern::Bool(_)
        | Pattern::Null => {}
    }
}

pub fn collect_pattern_binding_types(
    pattern: &Pattern,
    target_ty: &InferredType,
    type_info: &TypeInfo,
    types: &mut HashMap<String, InferredType>,
) {
    match pattern {
        Pattern::Bind(name) => {
            types.insert(name.clone(), target_ty.clone());
        }
        Pattern::List(parts, _) => {
            let item_ty = match target_ty {
                InferredType::List(inner) => (**inner).clone(),
                _ => InferredType::Dynamic,
            };
            for pat in parts {
                collect_pattern_binding_types(pat, &item_ty, type_info, types);
            }
        }
        Pattern::Struct(name, fields) => match target_ty {
            InferredType::Struct(struct_name) if struct_name == name => {
                for (field, pat) in fields {
                    let field_ty = type_info
                        .struct_fields
                        .get(struct_name)
                        .and_then(|m| m.get(field))
                        .cloned()
                        .unwrap_or(InferredType::Dynamic);
                    collect_pattern_binding_types(pat, &field_ty, type_info, types);
                }
            }
            _ => {
                for (_, pat) in fields {
                    collect_pattern_binding_types(pat, &InferredType::Dynamic, type_info, types);
                }
            }
        },
        Pattern::EnumVariant(name, parts) => match target_ty {
            InferredType::Enum(enum_name) => {
                let key = format!("{}.{}", enum_name, name);
                let field_tys = type_info
                    .enum_variant_fields
                    .get(&key)
                    .cloned()
                    .unwrap_or_default();
                for (idx, pat) in parts.iter().enumerate() {
                    let ty = field_tys.get(idx).cloned().unwrap_or(InferredType::Dynamic);
                    collect_pattern_binding_types(pat, &ty, type_info, types);
                }
            }
            _ => {
                for pat in parts {
                    collect_pattern_binding_types(pat, &InferredType::Dynamic, type_info, types);
                }
            }
        },
        Pattern::Wildcard
        | Pattern::Number(_)
        | Pattern::StringLit(_)
        | Pattern::Bool(_)
        | Pattern::Null => {}
    }
}

pub fn lower_pattern_checks(
    pattern: &Pattern,
    target_ty: &InferredType,
    type_info: &TypeInfo,
) -> Vec<LoweredPatternCheck> {
    let mut checks = Vec::new();
    lower_pattern_checks_at(pattern, &[], target_ty, type_info, &mut checks);
    checks
}

fn lower_pattern_checks_at(
    pattern: &Pattern,
    path: &[PatternAccessStep],
    target_ty: &InferredType,
    type_info: &TypeInfo,
    checks: &mut Vec<LoweredPatternCheck>,
) {
    match pattern {
        Pattern::Wildcard | Pattern::Bind(_) => {}
        Pattern::Number(value) => checks.push(LoweredPatternCheck {
            path: path.to_vec(),
            input_ty: target_ty.clone(),
            kind: LoweredPatternCheckKind::Number(*value),
        }),
        Pattern::StringLit(value) => checks.push(LoweredPatternCheck {
            path: path.to_vec(),
            input_ty: target_ty.clone(),
            kind: LoweredPatternCheckKind::StringLit(value.clone()),
        }),
        Pattern::Bool(value) => checks.push(LoweredPatternCheck {
            path: path.to_vec(),
            input_ty: target_ty.clone(),
            kind: LoweredPatternCheckKind::Bool(*value),
        }),
        Pattern::Null => checks.push(LoweredPatternCheck {
            path: path.to_vec(),
            input_ty: target_ty.clone(),
            kind: LoweredPatternCheckKind::Null,
        }),
        Pattern::List(parts, has_rest) => match target_ty {
            InferredType::List(inner) => {
                checks.push(LoweredPatternCheck {
                    path: path.to_vec(),
                    input_ty: target_ty.clone(),
                    kind: LoweredPatternCheckKind::ListLength {
                        len: parts.len(),
                        op: if *has_rest {
                            PatternLengthCheckOp::AtLeast
                        } else {
                            PatternLengthCheckOp::Exact
                        },
                    },
                });
                for (idx, part) in parts.iter().enumerate() {
                    let mut child_path = path.to_vec();
                    child_path.push(PatternAccessStep::ListIndex(idx));
                    lower_pattern_checks_at(part, &child_path, inner, type_info, checks);
                }
            }
            InferredType::Dynamic => {
                checks.push(LoweredPatternCheck {
                    path: path.to_vec(),
                    input_ty: target_ty.clone(),
                    kind: LoweredPatternCheckKind::ListLength {
                        len: parts.len(),
                        op: if *has_rest {
                            PatternLengthCheckOp::AtLeast
                        } else {
                            PatternLengthCheckOp::Exact
                        },
                    },
                });
                for (idx, part) in parts.iter().enumerate() {
                    let mut child_path = path.to_vec();
                    child_path.push(PatternAccessStep::ListIndex(idx));
                    lower_pattern_checks_at(part, &child_path, &InferredType::Dynamic, type_info, checks);
                }
            }
            _ => checks.push(LoweredPatternCheck {
                path: path.to_vec(),
                input_ty: target_ty.clone(),
                kind: LoweredPatternCheckKind::AlwaysFalse,
            }),
        },
        Pattern::Struct(name, fields) => match target_ty {
            InferredType::Struct(struct_name) if struct_name == name => {
                checks.push(LoweredPatternCheck {
                    path: path.to_vec(),
                    input_ty: target_ty.clone(),
                    kind: LoweredPatternCheckKind::StructType(name.clone()),
                });
                for (field, part) in fields {
                    let field_ty = type_info
                        .struct_fields
                        .get(struct_name)
                        .and_then(|m| m.get(field))
                        .cloned()
                        .unwrap_or(InferredType::Dynamic);
                    let mut child_path = path.to_vec();
                    child_path.push(PatternAccessStep::StructField(field.clone()));
                    lower_pattern_checks_at(part, &child_path, &field_ty, type_info, checks);
                }
            }
            InferredType::Dynamic => {
                checks.push(LoweredPatternCheck {
                    path: path.to_vec(),
                    input_ty: target_ty.clone(),
                    kind: LoweredPatternCheckKind::StructType(name.clone()),
                });
                for (field, part) in fields {
                    let mut child_path = path.to_vec();
                    child_path.push(PatternAccessStep::StructField(field.clone()));
                    lower_pattern_checks_at(part, &child_path, &InferredType::Dynamic, type_info, checks);
                }
            }
            _ => checks.push(LoweredPatternCheck {
                path: path.to_vec(),
                input_ty: target_ty.clone(),
                kind: LoweredPatternCheckKind::AlwaysFalse,
            }),
        },
        Pattern::EnumVariant(name, parts) => {
            let owner = type_info.variant_to_enum.get(name).cloned();
            checks.push(LoweredPatternCheck {
                path: path.to_vec(),
                input_ty: target_ty.clone(),
                kind: LoweredPatternCheckKind::EnumVariant {
                    enum_name: owner.clone(),
                    variant: name.clone(),
                    arity: parts.len(),
                },
            });

            match target_ty {
                InferredType::Enum(enum_name) if owner.as_ref() == Some(enum_name) => {
                    let key = format!("{}.{}", enum_name, name);
                    let field_tys = type_info
                        .enum_variant_fields
                        .get(&key)
                        .cloned()
                        .unwrap_or_default();
                    for (idx, part) in parts.iter().enumerate() {
                        let part_ty = field_tys.get(idx).cloned().unwrap_or(InferredType::Dynamic);
                        let mut child_path = path.to_vec();
                        child_path.push(PatternAccessStep::EnumField(name.clone(), idx));
                        lower_pattern_checks_at(part, &child_path, &part_ty, type_info, checks);
                    }
                }
                InferredType::Dynamic => {
                    for (idx, part) in parts.iter().enumerate() {
                        let mut child_path = path.to_vec();
                        child_path.push(PatternAccessStep::EnumField(name.clone(), idx));
                        lower_pattern_checks_at(part, &child_path, &InferredType::Dynamic, type_info, checks);
                    }
                }
                _ => {}
            }
        }
    }
}

pub fn infer_pattern_path_type(
    root_ty: &InferredType,
    path: &[PatternAccessStep],
    type_info: &TypeInfo,
) -> InferredType {
    let mut current = root_ty.clone();
    for step in path {
        current = match step {
            PatternAccessStep::StructField(field) => match &current {
                InferredType::Struct(struct_name) => type_info
                    .struct_fields
                    .get(struct_name)
                    .and_then(|fields| fields.get(field))
                    .cloned()
                    .unwrap_or(InferredType::Dynamic),
                InferredType::Dynamic => InferredType::Dynamic,
                _ => InferredType::Dynamic,
            },
            PatternAccessStep::EnumField(variant, idx) => match &current {
                InferredType::Enum(enum_name) => type_info
                    .enum_variant_fields
                    .get(&format!("{}.{}", enum_name, variant))
                    .and_then(|fields| fields.get(*idx))
                    .cloned()
                    .unwrap_or(InferredType::Dynamic),
                InferredType::Dynamic => InferredType::Dynamic,
                _ => InferredType::Dynamic,
            },
            PatternAccessStep::ListIndex(_) => match &current {
                InferredType::List(inner) => (**inner).clone(),
                InferredType::Dynamic => InferredType::Dynamic,
                _ => InferredType::Dynamic,
            },
        };
    }
    current
}

pub fn lower_pattern_bindings(
    pattern: &Pattern,
    target_ty: &InferredType,
    type_info: &TypeInfo,
) -> Vec<LoweredPatternBinding> {
    collect_pattern_binding_paths(pattern)
        .into_iter()
        .map(|binding| LoweredPatternBinding {
            ty: infer_pattern_path_type(target_ty, &binding.path, type_info),
            name: binding.name,
            path: binding.path,
        })
        .collect()
}

pub fn lower_destructure(
    pattern: &Pattern,
    target_ty: &InferredType,
    type_info: &TypeInfo,
) -> LoweredDestructure {
    LoweredDestructure {
        target_ty: target_ty.clone(),
        checks: lower_pattern_checks(pattern, target_ty, type_info),
        bindings: lower_pattern_bindings(pattern, target_ty, type_info),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_nested_binding_paths() {
        let pattern = Pattern::Struct(
            "Pair".to_string(),
            vec![
                ("left".to_string(), Pattern::Bind("a".to_string())),
                (
                    "right".to_string(),
                    Pattern::EnumVariant(
                        "Some".to_string(),
                        vec![Pattern::List(vec![Pattern::Bind("b".to_string())], false)],
                    ),
                ),
            ],
        );

        let bindings = collect_pattern_binding_paths(&pattern);
        assert_eq!(
            bindings,
            vec![
                PatternBinding {
                    name: "a".to_string(),
                    path: vec![PatternAccessStep::StructField("left".to_string())],
                },
                PatternBinding {
                    name: "b".to_string(),
                    path: vec![
                        PatternAccessStep::StructField("right".to_string()),
                        PatternAccessStep::EnumField("Some".to_string(), 0),
                        PatternAccessStep::ListIndex(0),
                    ],
                },
            ]
        );
    }

    #[test]
    fn lowers_nested_pattern_checks_with_types() {
        let mut type_info = TypeInfo::default();
        type_info
            .variant_to_enum
            .insert("Some".to_string(), "Option".to_string());
        type_info.enum_variant_fields.insert(
            "Option.Some".to_string(),
            vec![InferredType::Struct("User".to_string())],
        );
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), InferredType::String);
        type_info.struct_fields.insert("User".to_string(), fields);

        let pattern = Pattern::EnumVariant(
            "Some".to_string(),
            vec![Pattern::Struct(
                "User".to_string(),
                vec![("name".to_string(), Pattern::StringLit("Ana".to_string()))],
            )],
        );

        let checks = lower_pattern_checks(
            &pattern,
            &InferredType::Enum("Option".to_string()),
            &type_info,
        );

        assert!(matches!(
            checks[0].kind,
            LoweredPatternCheckKind::EnumVariant {
                ref enum_name,
                ref variant,
                arity: 1
            } if enum_name.as_deref() == Some("Option") && variant == "Some"
        ));
        assert!(matches!(
            &checks[1],
            LoweredPatternCheck {
                path,
                input_ty: InferredType::Struct(name),
                kind: LoweredPatternCheckKind::StructType(struct_name),
            } if *path == vec![PatternAccessStep::EnumField("Some".to_string(), 0)]
                && name == "User"
                && struct_name == "User"
        ));
    }

    #[test]
    fn lowers_destructure_bindings_with_types() {
        let mut type_info = TypeInfo::default();
        type_info
            .variant_to_enum
            .insert("Some".to_string(), "Option".to_string());
        type_info.enum_variant_fields.insert(
            "Option.Some".to_string(),
            vec![InferredType::Struct("User".to_string())],
        );
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), InferredType::String);
        type_info.struct_fields.insert("User".to_string(), fields);

        let pattern = Pattern::EnumVariant(
            "Some".to_string(),
            vec![Pattern::Struct(
                "User".to_string(),
                vec![("name".to_string(), Pattern::Bind("name".to_string()))],
            )],
        );

        let lowered = lower_destructure(
            &pattern,
            &InferredType::Enum("Option".to_string()),
            &type_info,
        );

        assert_eq!(lowered.bindings.len(), 1);
        assert_eq!(lowered.bindings[0].name, "name");
        assert_eq!(lowered.bindings[0].ty, InferredType::String);
        assert_eq!(
            lowered.bindings[0].path,
            vec![
                PatternAccessStep::EnumField("Some".to_string(), 0),
                PatternAccessStep::StructField("name".to_string()),
            ]
        );
    }
}
