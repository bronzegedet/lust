use crate::ast::Expr;
use crate::dispatch::{
    classify_method_call, infer_expr_type, rewrite_pipe_expr, BuiltinMethodKind, MethodDispatch,
};
use crate::typecheck::{InferredType, TypeInfo};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub enum LoweredCall {
    BuiltinFunction {
        name: String,
        args: Vec<Expr>,
    },
    UserFunction {
        name: String,
        args: Vec<Expr>,
    },
    BuiltinMethod {
        receiver: Expr,
        kind: BuiltinMethodKind,
        args: Vec<Expr>,
    },
    StructMethod {
        receiver: Expr,
        method_name: String,
        key: String,
        args: Vec<Expr>,
    },
    DynamicMethod {
        receiver: Expr,
        method_name: String,
        args: Vec<Expr>,
    },
    DynamicCall {
        target: Expr,
        args: Vec<Expr>,
    },
}

pub fn lower_call_expr(
    expr: &Expr,
    local_types: &HashMap<String, InferredType>,
    type_info: &TypeInfo,
    globals: &HashSet<String>,
) -> Option<LoweredCall> {
    match expr {
        Expr::Pipe(target, name, args) => {
            let target_ty = infer_expr_type(target, local_types, type_info, globals);
            let rewritten = rewrite_pipe_expr(target, name, args, &target_ty, type_info);
            lower_call_expr(&rewritten, local_types, type_info, globals)
        }
        Expr::Call(name, args) => {
            if is_builtin_function(name) {
                Some(LoweredCall::BuiltinFunction {
                    name: name.clone(),
                    args: args.clone(),
                })
            } else if type_info.signatures.contains_key(name) {
                Some(LoweredCall::UserFunction {
                    name: name.clone(),
                    args: args.clone(),
                })
            } else {
                // Potential dynamic call to a variable containing a function
                Some(LoweredCall::DynamicCall {
                    target: Expr::Ident(name.clone()),
                    args: args.clone(),
                })
            }
        }
        Expr::MethodCall(receiver, method_name, args) => {
            let receiver_ty = infer_expr_type(receiver, local_types, type_info, globals);
            match classify_method_call(&receiver_ty, method_name, type_info) {
                MethodDispatch::ListPush => Some(LoweredCall::DynamicMethod {
                    receiver: (**receiver).clone(),
                    method_name: method_name.clone(),
                    args: args.clone(),
                }),
                MethodDispatch::Builtin(kind) => Some(LoweredCall::BuiltinMethod {
                    receiver: (**receiver).clone(),
                    kind,
                    args: args.clone(),
                }),
                MethodDispatch::StructMethod { key } => Some(LoweredCall::StructMethod {
                    receiver: (**receiver).clone(),
                    method_name: method_name.clone(),
                    key,
                    args: args.clone(),
                }),
                MethodDispatch::DynamicValueMethod => Some(LoweredCall::DynamicMethod {
                    receiver: (**receiver).clone(),
                    method_name: method_name.clone(),
                    args: args.clone(),
                }),
            }
        }
        _ => None,
    }
}

fn is_builtin_function(name: &str) -> bool {
    matches!(
        name,
        "println"
            | "to_string"
            | "to_number"
            | "type_of"
            | "debug"
            | "panic"
            | "assert"
            | "read_file"
            | "read_file_result"
            | "try_read_file"
            | "write_file"
            | "append_file"
            | "open_file"
            | "list_dir"
            | "get_args"
            | "launch_lust"
            | "input"
            | "get_env"
            | "get_key"
            | "poll_key"
            | "json_encode"
            | "json_decode"
            | "json_parse"
            | "sleep"
            | "now"
            | "random_int"
            | "random_float"
            | "audio_init"
            | "audio_set_freq"
            | "audio_set_gain"
            | "audio_note_on"
            | "audio_note_off"
            | "window"
            | "live"
            | "clear_screen"
            | "circle"
            | "rect"
            | "line"
            | "triangle"
            | "text"
            | "ui_knob"
            | "ui_slider"
            | "ui_toggle"
            | "ui_textbox"
            | "ui_button"
            | "ui_theme"
            | "ui_set"
            | "ui_get"
            | "ui_caret"
            | "ui_selection_start"
            | "ui_selection_end"
            | "ui_scroll_y"
            | "ui_text_input"
            | "ui_command"
            | "ui_key_left"
            | "ui_key_right"
            | "ui_key_up"
            | "ui_key_down"
            | "ui_key_enter"
            | "ui_key_esc"
            | "ui_key_backspace"
            | "ui_key_delete"
            | "ui_mouse_x"
            | "ui_mouse_y"
            | "ui_mouse_down"
            | "ui_mouse_clicked"
            | "ui_mouse_click_x"
            | "ui_mouse_click_y"
            | "filter"
            | "map"
            | "__range"
            | "__range_inclusive"
            | "clr"
            | "prompt"
            | "dict"
            | "map_values"
            | "filter_values"
            | "map_entries"
            | "filter_entries"
            | "compile_lustgex"
            | "regex_capture"
            | "lustgex_match"
            | "lustgex_capture_builtin"
            | "__str_insert"
            | "__str_delete_range"
            | "__str_find"
            | "__slice_range"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typecheck::{FunctionSig, TypeInfo};

    #[test]
    fn lowers_pipe_to_builtin_method_call() {
        let expr = Expr::Pipe(
            Box::new(Expr::StringLit(" hi ".to_string())),
            "trim".to_string(),
            vec![],
        );
        let lowered = lower_call_expr(&expr, &HashMap::new(), &TypeInfo::default(), &HashSet::new())
            .expect("expected lowered call");
        assert!(matches!(
            lowered,
            LoweredCall::BuiltinMethod {
                kind: BuiltinMethodKind::Trim,
                ..
            }
        ));
    }

    #[test]
    fn lowers_struct_method_call_to_struct_method_node() {
        let mut type_info = TypeInfo::default();
        type_info.signatures.insert(
            "User.rename".to_string(),
            FunctionSig {
                params: vec![InferredType::String],
                ret: InferredType::String,
            },
        );
        let mut locals = HashMap::new();
        locals.insert("user".to_string(), InferredType::Struct("User".to_string()));
        let expr = Expr::MethodCall(
            Box::new(Expr::Ident("user".to_string())),
            "rename".to_string(),
            vec![Expr::StringLit("Dr. ".to_string())],
        );
        let lowered = lower_call_expr(&expr, &locals, &type_info, &HashSet::new())
            .expect("expected lowered call");
        assert!(matches!(
            lowered,
            LoweredCall::StructMethod { ref key, .. } if key == "User.rename"
        ));
    }
}
