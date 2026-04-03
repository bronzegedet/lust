use crate::typecheck::InferredType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinArgKind {
    String,
    Number,
    Any,
}

pub fn infer_builtin_method_type(target_ty: &InferredType, name: &str) -> InferredType {
    match (target_ty, name) {
        (_, "to_string") => InferredType::String,
        (_, "contains") | (_, "starts_with") | (_, "ends_with") | (_, "is_alphabetic") | (_, "is_numeric") => InferredType::Boolean,
        (_, "length") => InferredType::Int,
        (InferredType::Map(_), "keys") => InferredType::List(Box::new(InferredType::String)),
        (InferredType::Map(inner), "values") => InferredType::List(inner.clone()),
        (InferredType::Map(_), "entries") => InferredType::List(Box::new(InferredType::Dynamic)),
        (InferredType::Map(_), "has") => InferredType::Boolean,
        (InferredType::Map(inner), "map_values") => InferredType::Map(inner.clone()),
        (InferredType::Map(inner), "filter_values") => InferredType::Map(inner.clone()),
        (InferredType::Map(_), "map_entries") => InferredType::Map(Box::new(InferredType::Dynamic)),
        (InferredType::Map(inner), "filter_entries") => InferredType::Map(inner.clone()),
        (InferredType::String, "split")
        | (InferredType::String, "to_list")
        | (InferredType::String, "to_chars")
        | (InferredType::String, "lines") => InferredType::List(Box::new(InferredType::String)),
        (InferredType::String, "trim")
        | (InferredType::String, "slice")
        | (InferredType::String, "at")
        | (InferredType::String, "replace") => InferredType::String,
        (InferredType::Struct(_), "get_field") => InferredType::Dynamic,
        (InferredType::Struct(_), "set_field") => InferredType::Dynamic,
        _ => InferredType::Dynamic,
    }
}

pub fn pipe_prefers_method(target_ty: &InferredType, name: &str, has_struct_method: bool) -> bool {
    has_struct_method || infer_builtin_method_type(target_ty, name) != InferredType::Dynamic
}

pub fn builtin_method_requires_string_receiver(name: &str) -> bool {
    matches!(
        name,
        "trim"
            | "slice"
            | "at"
            | "split"
            | "contains"
            | "to_list"
            | "to_chars"
            | "lines"
            | "starts_with"
            | "ends_with"
            | "replace"
            | "is_alphabetic"
            | "is_numeric"
    )
}

pub fn builtin_method_arg_kinds(name: &str) -> Option<&'static [BuiltinArgKind]> {
    match name {
        "to_string" | "length" | "trim" | "to_list" | "to_chars" | "lines" | "is_alphabetic" | "is_numeric" | "keys" | "values" | "entries" => Some(&[]),
        "at" => Some(&[BuiltinArgKind::Number]),
        "slice" => Some(&[BuiltinArgKind::Number, BuiltinArgKind::Number]),
        "contains" | "split" | "starts_with" | "ends_with" | "has" => Some(&[BuiltinArgKind::String]),
        "map_values" | "filter_values" | "map_entries" | "filter_entries" => Some(&[BuiltinArgKind::Any]),
        "replace" => Some(&[BuiltinArgKind::String, BuiltinArgKind::String]),
        "push" => None,
        "get_field" => Some(&[BuiltinArgKind::String]),
        "set_field" => Some(&[BuiltinArgKind::String, BuiltinArgKind::Any]),
        _ => None,
    }
}

pub fn is_builtin_method(name: &str) -> bool {
    builtin_method_arg_kinds(name).is_some()
}
