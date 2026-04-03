use crate::ast::Expr;
use crate::typecheck::{InferredType, TypeInfo};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinMethodKind {
    ToString,
    Length,
    Trim,
    At,
    Slice,
    Contains,
    Split,
    ToList,
    Lines,
    StartsWith,
    EndsWith,
    Replace,
    Keys,
    Values,
    Entries,
    Has,
    MapValues,
    FilterValues,
    MapEntries,
    FilterEntries,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MethodDispatch {
    ListPush,
    Builtin(BuiltinMethodKind),
    StructMethod {
        key: String,
    },
    DynamicValueMethod,
}

pub fn infer_call_type(name: &str) -> InferredType {
    match name {
        "sin" | "cos" | "sqrt" | "abs" | "random_float" | "now" | "to_number" => {
            InferredType::Number
        }
        "random_int" => InferredType::Int,
        "__range" | "__range_inclusive" => InferredType::List(Box::new(InferredType::Number)),
        "dict" => InferredType::Map(Box::new(InferredType::Dynamic)),
        "map_values" | "filter_values" | "map_entries" | "filter_entries" => InferredType::Map(Box::new(InferredType::Dynamic)),
        "get_key" => InferredType::Enum("Key".to_string()),
        "poll_key" => InferredType::Dynamic,
        "live" => InferredType::Boolean,
        "ui_knob" | "ui_slider" => InferredType::Number,
        "ui_toggle" | "ui_button" | "ui_key_left" | "ui_key_right" | "ui_key_up"
        | "ui_key_down" | "ui_key_enter" | "ui_key_esc" | "ui_key_backspace"
        | "ui_key_delete" | "ui_mouse_down" | "ui_mouse_clicked" => InferredType::Boolean,
        "ui_theme" => InferredType::String,
        "ui_textbox" | "ui_text_input" | "ui_command" => InferredType::String,
        "ui_caret" | "ui_selection_start" | "ui_selection_end" | "ui_scroll_y"
        | "ui_mouse_x" | "ui_mouse_y" | "ui_mouse_click_x" | "ui_mouse_click_y" => {
            InferredType::Number
        }
        "ui_set" | "ui_get" => InferredType::Dynamic,
        "to_string" | "type_of" | "read_file" | "try_read_file" | "json_encode" | "json_decode" | "compile_lustgex"
        | "get_env" | "string_builder_build" => InferredType::String,
        "__str_insert" | "__str_delete_range" => InferredType::String,
        "__str_find" => InferredType::Int,
        "__slice_range" => InferredType::Dynamic,
        "read_file_result" => InferredType::Enum("FileResult".to_string()),
        "list_dir" => InferredType::List(Box::new(InferredType::String)),
        "json_parse" => InferredType::Dynamic,
        "clr" => InferredType::Dynamic,
        "prompt" => InferredType::String,
        "lustgex_match" => InferredType::Boolean,
        "regex_capture" | "lustgex_capture_builtin" | "create_string_builder" | "string_builder_append" => InferredType::Dynamic,
        "launch_lust" => InferredType::Int,
        "open_file" => InferredType::Dynamic,
        "append_file" | "debug" | "panic" | "assert" => InferredType::Dynamic,
        _ => InferredType::Dynamic,
    }
}

pub fn builtin_method_kind(name: &str) -> Option<BuiltinMethodKind> {
    match name {
        "to_string" => Some(BuiltinMethodKind::ToString),
        "length" => Some(BuiltinMethodKind::Length),
        "trim" => Some(BuiltinMethodKind::Trim),
        "at" => Some(BuiltinMethodKind::At),
        "slice" => Some(BuiltinMethodKind::Slice),
        "contains" => Some(BuiltinMethodKind::Contains),
        "split" => Some(BuiltinMethodKind::Split),
        "to_list" => Some(BuiltinMethodKind::ToList),
        "lines" => Some(BuiltinMethodKind::Lines),
        "starts_with" => Some(BuiltinMethodKind::StartsWith),
        "ends_with" => Some(BuiltinMethodKind::EndsWith),
        "replace" => Some(BuiltinMethodKind::Replace),
        "keys" => Some(BuiltinMethodKind::Keys),
        "values" => Some(BuiltinMethodKind::Values),
        "entries" => Some(BuiltinMethodKind::Entries),
        "has" => Some(BuiltinMethodKind::Has),
        "map_values" => Some(BuiltinMethodKind::MapValues),
        "filter_values" => Some(BuiltinMethodKind::FilterValues),
        "map_entries" => Some(BuiltinMethodKind::MapEntries),
        "filter_entries" => Some(BuiltinMethodKind::FilterEntries),
        _ => None,
    }
}

impl BuiltinMethodKind {
    pub fn expected_args(self) -> usize {
        match self {
            BuiltinMethodKind::ToString
            | BuiltinMethodKind::Length
            | BuiltinMethodKind::Trim
            | BuiltinMethodKind::ToList
            | BuiltinMethodKind::Lines
            | BuiltinMethodKind::Keys
            | BuiltinMethodKind::Values
            | BuiltinMethodKind::Entries => 0,
            BuiltinMethodKind::At
            | BuiltinMethodKind::Contains
            | BuiltinMethodKind::Split
            | BuiltinMethodKind::StartsWith
            | BuiltinMethodKind::EndsWith
            | BuiltinMethodKind::Has
            | BuiltinMethodKind::MapValues
            | BuiltinMethodKind::FilterValues
            | BuiltinMethodKind::MapEntries
            | BuiltinMethodKind::FilterEntries => 1,
            BuiltinMethodKind::Slice | BuiltinMethodKind::Replace => 2,
        }
    }

    pub fn vm_builtin_name(self) -> Option<&'static str> {
        match self {
            BuiltinMethodKind::ToString => Some("to_string"),
            BuiltinMethodKind::Trim => Some("__str_trim"),
            BuiltinMethodKind::At => Some("__str_at"),
            BuiltinMethodKind::Slice => Some("__str_slice"),
            BuiltinMethodKind::Contains => Some("__str_contains"),
            BuiltinMethodKind::Split => Some("__str_split"),
            BuiltinMethodKind::ToList => Some("__str_to_list"),
            BuiltinMethodKind::Lines => Some("__str_lines"),
            BuiltinMethodKind::StartsWith => Some("__str_starts_with"),
            BuiltinMethodKind::EndsWith => Some("__str_ends_with"),
            BuiltinMethodKind::Replace => Some("__str_replace"),
            BuiltinMethodKind::Keys => Some("__map_keys"),
            BuiltinMethodKind::Values => Some("__map_values"),
            BuiltinMethodKind::Entries => Some("__map_entries"),
            BuiltinMethodKind::Has => Some("__map_has"),
            BuiltinMethodKind::MapValues => Some("map_values"),
            BuiltinMethodKind::FilterValues => Some("filter_values"),
            BuiltinMethodKind::MapEntries => Some("map_entries"),
            BuiltinMethodKind::FilterEntries => Some("filter_entries"),
            BuiltinMethodKind::Length => None,
        }
    }
}

pub fn has_struct_method(type_info: &TypeInfo, target_ty: &InferredType, name: &str) -> bool {
    matches!(target_ty, InferredType::Struct(struct_name) if type_info.signatures.contains_key(&format!("{}.{}", struct_name, name)))
}

pub fn rewrite_pipe_expr(
    target: &Expr,
    name: &str,
    args: &[Expr],
    target_ty: &InferredType,
    type_info: &TypeInfo,
) -> Expr {
    if crate::helpers::pipe_prefers_method(target_ty, name, has_struct_method(type_info, target_ty, name)) {
        Expr::MethodCall(Box::new(target.clone()), name.to_string(), args.to_vec())
    } else {
        let mut call_args = vec![target.clone()];
        call_args.extend(args.iter().cloned());
        Expr::Call(name.to_string(), call_args)
    }
}

pub fn infer_method_return_type(target_ty: &InferredType, name: &str, type_info: &TypeInfo) -> InferredType {
    let builtin_ty = crate::helpers::infer_builtin_method_type(target_ty, name);
    if builtin_ty != InferredType::Dynamic {
        builtin_ty
    } else if let InferredType::Struct(struct_name) = target_ty {
        type_info
            .signatures
            .get(&format!("{}.{}", struct_name, name))
            .map(|s| s.ret.clone())
            .unwrap_or(InferredType::Dynamic)
    } else {
        InferredType::Dynamic
    }
}

pub fn classify_method_call(target_ty: &InferredType, name: &str, type_info: &TypeInfo) -> MethodDispatch {
    match name {
        "push" => MethodDispatch::ListPush,
        _ => {
            if let Some(kind) = builtin_method_kind(name) {
                MethodDispatch::Builtin(kind)
            } else if let InferredType::Struct(struct_name) = target_ty {
                let key = format!("{}.{}", struct_name, name);
                if type_info.signatures.contains_key(&key) {
                    MethodDispatch::StructMethod { key }
                } else {
                    MethodDispatch::DynamicValueMethod
                }
            } else {
                MethodDispatch::DynamicValueMethod
            }
        }
    }
}

pub fn infer_expr_type(
    expr: &Expr,
    local_types: &HashMap<String, InferredType>,
    type_info: &TypeInfo,
    globals: &std::collections::HashSet<String>,
) -> InferredType {
    match expr {
        Expr::Number(value) => {
            if value.is_finite() && value.fract() == 0.0 {
                InferredType::Int
            } else {
                InferredType::Number
            }
        }
        Expr::StringLit(_) => InferredType::String,
        Expr::Lambda(_, _) => InferredType::Dynamic,
        Expr::Pipe(target, name, args) => {
            let target_ty = infer_expr_type(target, local_types, type_info, globals);
            let rewritten = rewrite_pipe_expr(target, name, args, &target_ty, type_info);
            infer_expr_type(&rewritten, local_types, type_info, globals)
        }
        Expr::List(items) => {
            let mut elem_ty = InferredType::Dynamic;
            for item in items {
                let item_ty = infer_expr_type(item, local_types, type_info, globals);
                elem_ty = unify_types(&elem_ty, &item_ty);
            }
            match elem_ty {
                InferredType::Dynamic => InferredType::Dynamic,
                _ => InferredType::List(Box::new(elem_ty)),
            }
        }
        Expr::MapLit(items) => {
            let mut value_ty = InferredType::Dynamic;
            for (_, value) in items {
                value_ty = unify_types(&value_ty, &infer_expr_type(value, local_types, type_info, globals));
            }
            InferredType::Map(Box::new(value_ty))
        }
        Expr::Ident(id) => {
            if id == "true" || id == "false" {
                InferredType::Boolean
            } else if id == "null" || globals.contains(id) {
                InferredType::Dynamic
            } else {
                local_types.get(id).cloned().unwrap_or(InferredType::Dynamic)
            }
        }
        Expr::Self_ => local_types.get("self").cloned().unwrap_or(InferredType::Dynamic),
        Expr::Binary(left, op, right) => {
            let left_ty = infer_expr_type(left, local_types, type_info, globals);
            let right_ty = infer_expr_type(right, local_types, type_info, globals);
            match op.as_str() {
                "+" => {
                    if is_numeric_type(&left_ty) && is_numeric_type(&right_ty) {
                        if left_ty == InferredType::Int && right_ty == InferredType::Int {
                            InferredType::Int
                        } else {
                            InferredType::Number
                        }
                    } else if left_ty == InferredType::String && right_ty == InferredType::String {
                        InferredType::String
                    } else {
                        InferredType::Dynamic
                    }
                }
                "-" | "*" | "%" => {
                    if is_numeric_type(&left_ty) && is_numeric_type(&right_ty) {
                        if left_ty == InferredType::Int && right_ty == InferredType::Int {
                            InferredType::Int
                        } else {
                            InferredType::Number
                        }
                    } else {
                        InferredType::Dynamic
                    }
                }
                "/" => {
                    if is_numeric_type(&left_ty) && is_numeric_type(&right_ty) {
                        InferredType::Number
                    } else {
                        InferredType::Dynamic
                    }
                }
                "==" | "!=" | ">" | "<" | ">=" | "<=" | "and" | "or" | "not" => InferredType::Boolean,
                _ => InferredType::Dynamic,
            }
        }
        Expr::Call(name, args) => match name.as_str() {
            "dict" => {
                let mut value_ty = InferredType::Dynamic;
                for value in args.iter().skip(1).step_by(2) {
                    value_ty = unify_types(&value_ty, &infer_expr_type(value, local_types, type_info, globals));
                }
                InferredType::Map(Box::new(value_ty))
            }
            "map_values" => {
                if let (Some(source), Some(Expr::Lambda(params, body))) = (args.get(0), args.get(1)) {
                    if let InferredType::Map(inner) = infer_expr_type(source, local_types, type_info, globals) {
                        if params.len() == 1 {
                            let mut lambda_locals = local_types.clone();
                            lambda_locals.insert(params[0].clone(), (*inner).clone());
                            let ret_ty = infer_expr_type(body, &lambda_locals, type_info, globals);
                            return InferredType::Map(Box::new(ret_ty));
                        }
                    }
                }
                InferredType::Dynamic
            }
            "map_entries" => InferredType::Map(Box::new(InferredType::Dynamic)),
            "filter_values" => {
                if let Some(source) = args.first() {
                    if let InferredType::Map(inner) = infer_expr_type(source, local_types, type_info, globals) {
                        InferredType::Map(inner)
                    } else {
                        InferredType::Dynamic
                    }
                } else {
                    InferredType::Dynamic
                }
            }
            "filter_entries" => {
                if let Some(source) = args.first() {
                    if let InferredType::Map(inner) = infer_expr_type(source, local_types, type_info, globals) {
                        InferredType::Map(inner)
                    } else {
                        InferredType::Dynamic
                    }
                } else {
                    InferredType::Dynamic
                }
            }
            "filter" => {
                if let Some(source) = args.first() {
                    if let InferredType::List(inner) = infer_expr_type(source, local_types, type_info, globals) {
                        InferredType::List(inner)
                    } else {
                        InferredType::Dynamic
                    }
                } else {
                    InferredType::Dynamic
                }
            }
            "map" => {
                if let (Some(source), Some(Expr::Lambda(params, body))) = (args.get(0), args.get(1)) {
                    if let InferredType::List(inner) = infer_expr_type(source, local_types, type_info, globals) {
                        if params.len() == 1 {
                            let mut lambda_locals = local_types.clone();
                            lambda_locals.insert(params[0].clone(), (*inner).clone());
                            let ret_ty = infer_expr_type(body, &lambda_locals, type_info, globals);
                            return InferredType::List(Box::new(ret_ty));
                        }
                    }
                }
                InferredType::Dynamic
            }
            _ => type_info
                .signatures
                .get(name)
                .map(|s| s.ret.clone())
                .unwrap_or_else(|| infer_call_type(name)),
        },
        Expr::MethodCall(obj, name, _) => {
            let target_ty = infer_expr_type(obj, local_types, type_info, globals);
            infer_method_return_type(&target_ty, name, type_info)
        }
        Expr::StructInst(name, _, _) => InferredType::Struct(name.clone()),
        Expr::EnumVariant(name, _) => type_info
            .variant_to_enum
            .get(name)
            .cloned()
            .map(InferredType::Enum)
            .unwrap_or(InferredType::Dynamic),
        Expr::Index(obj, _) => match infer_expr_type(obj, local_types, type_info, globals) {
            InferredType::List(inner) => (*inner).clone(),
            InferredType::Map(inner) => (*inner).clone(),
            InferredType::String => InferredType::String,
            _ => InferredType::Dynamic,
        },
        Expr::Slice(obj, _, _) => match infer_expr_type(obj, local_types, type_info, globals) {
            InferredType::List(inner) => InferredType::List(inner),
            InferredType::String => InferredType::String,
            _ => InferredType::Dynamic,
        },
        Expr::Member(target, field) => {
            if let Expr::Ident(enum_name) = target.as_ref() {
                if type_info
                    .variant_to_enum
                    .get(field)
                    .map(|owner| owner == enum_name)
                    .unwrap_or(false)
                {
                    return InferredType::Enum(enum_name.clone());
                }
            }
            if let InferredType::Struct(name) = infer_expr_type(target, local_types, type_info, globals) {
                return type_info
                    .struct_fields
                    .get(&name)
                    .and_then(|m| m.get(field))
                    .cloned()
                    .unwrap_or(InferredType::Dynamic);
            }
            InferredType::Dynamic
        }
    }
}

fn unify_types(left: &InferredType, right: &InferredType) -> InferredType {
    match (left, right) {
        (InferredType::Dynamic, other) | (other, InferredType::Dynamic) => other.clone(),
        (InferredType::Int, InferredType::Int) => InferredType::Int,
        (InferredType::Int, InferredType::Number)
        | (InferredType::Number, InferredType::Int)
        | (InferredType::Number, InferredType::Number) => InferredType::Number,
        (InferredType::String, InferredType::String) => InferredType::String,
        (InferredType::Boolean, InferredType::Boolean) => InferredType::Boolean,
        (InferredType::Struct(a), InferredType::Struct(b)) if a == b => InferredType::Struct(a.clone()),
        (InferredType::Enum(a), InferredType::Enum(b)) if a == b => InferredType::Enum(a.clone()),
        (InferredType::List(a), InferredType::List(b)) => {
            InferredType::List(Box::new(unify_types(a, b)))
        }
        (InferredType::Map(a), InferredType::Map(b)) => {
            InferredType::Map(Box::new(unify_types(a, b)))
        }
        _ => InferredType::Dynamic,
    }
}

fn is_numeric_type(value: &InferredType) -> bool {
    matches!(value, InferredType::Int | InferredType::Number)
}
