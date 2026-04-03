use crate::access::{lower_read_access, LoweredReadAccess};
use crate::ast::Expr;
use crate::dispatch::infer_expr_type;
use crate::lowered::{lower_call_expr, LoweredCall};
use crate::typecheck::{InferredType, TypeInfo};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionKind {
    Boolean,
    TruthyValue,
}

#[derive(Debug, Clone)]
pub struct LoweredConditionIr {
    pub expr: Expr,
    pub kind: ConditionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoweredBinaryExprKind {
    NumericArithmetic,
    StringConcat,
    NativeBooleanLogic,
    DynamicBooleanLogic,
    NativeComparison { operand_ty: InferredType },
    DynamicComparison,
    DynamicValue,
}

#[derive(Debug, Clone)]
pub struct LoweredBinaryExprIr {
    pub left: Expr,
    pub op: String,
    pub right: Expr,
    pub result_ty: InferredType,
    pub kind: LoweredBinaryExprKind,
}

#[derive(Debug, Clone)]
pub enum LoweredExprIr {
    Call(LoweredCall),
    Read(LoweredReadAccess),
    Binary(LoweredBinaryExprIr),
}

pub fn lower_expr_ir(
    expr: &Expr,
    local_types: &HashMap<String, InferredType>,
    type_info: &TypeInfo,
    globals: &HashSet<String>,
) -> Option<LoweredExprIr> {
    if let Some(lowered) = lower_binary_expr_ir(expr, local_types, type_info, globals) {
        return Some(LoweredExprIr::Binary(lowered));
    }
    if let Some(lowered) = lower_call_expr(expr, local_types, type_info, globals) {
        return Some(LoweredExprIr::Call(lowered));
    }
    if let Some(lowered) = lower_read_access(expr, local_types, type_info, globals) {
        return Some(LoweredExprIr::Read(lowered));
    }
    None
}

pub fn lower_condition_ir(
    expr: &Expr,
    local_types: &HashMap<String, InferredType>,
    type_info: &TypeInfo,
    globals: &HashSet<String>,
) -> LoweredConditionIr {
    let expr_type = infer_expr_type(expr, local_types, type_info, globals);
    let kind = if expr_type == InferredType::Boolean {
        ConditionKind::Boolean
    } else {
        ConditionKind::TruthyValue
    };
    LoweredConditionIr {
        expr: expr.clone(),
        kind,
    }
}

pub fn lower_binary_expr_ir(
    expr: &Expr,
    local_types: &HashMap<String, InferredType>,
    type_info: &TypeInfo,
    globals: &HashSet<String>,
) -> Option<LoweredBinaryExprIr> {
    let Expr::Binary(left, op, right) = expr else {
        return None;
    };

    let left_ty = infer_expr_type(left, local_types, type_info, globals);
    let right_ty = infer_expr_type(right, local_types, type_info, globals);
    let result_ty = infer_expr_type(expr, local_types, type_info, globals);
    let kind = match op.as_str() {
        "+" if is_numeric_type(&left_ty) && is_numeric_type(&right_ty) => {
            LoweredBinaryExprKind::NumericArithmetic
        }
        "+" if left_ty == InferredType::String && right_ty == InferredType::String => {
            LoweredBinaryExprKind::StringConcat
        }
        "-" | "*" | "/" | "%" if is_numeric_type(&left_ty) && is_numeric_type(&right_ty) => {
            LoweredBinaryExprKind::NumericArithmetic
        }
        "and" | "or" | "not" if left_ty == InferredType::Boolean && right_ty == InferredType::Boolean => {
            LoweredBinaryExprKind::NativeBooleanLogic
        }
        "and" | "or" | "not" => LoweredBinaryExprKind::DynamicBooleanLogic,
        "==" | "!=" | ">" | "<" | ">=" | "<=" => {
            let operand_ty = if left_ty == InferredType::Dynamic { right_ty.clone() } else { left_ty.clone() };
            if operand_ty == InferredType::Dynamic {
                LoweredBinaryExprKind::DynamicComparison
            } else {
                LoweredBinaryExprKind::NativeComparison { operand_ty }
            }
        }
        _ => LoweredBinaryExprKind::DynamicValue,
    };

    Some(LoweredBinaryExprIr {
        left: (*left.clone()),
        op: op.clone(),
        right: (*right.clone()),
        result_ty,
        kind,
    })
}

fn is_numeric_type(value: &InferredType) -> bool {
    matches!(value, InferredType::Int | InferredType::Number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::BuiltinMethodKind;

    #[test]
    fn lowers_pipe_expr_into_expr_ir_call() {
        let expr = Expr::Pipe(
            Box::new(Expr::StringLit(" hi ".to_string())),
            "trim".to_string(),
            vec![],
        );
        let lowered =
            lower_expr_ir(&expr, &HashMap::new(), &TypeInfo::default(), &HashSet::new())
                .expect("expected lowered expr ir");
        match lowered {
            LoweredExprIr::Call(LoweredCall::BuiltinMethod { kind, .. }) => {
                assert_eq!(kind, BuiltinMethodKind::Trim);
            }
            _ => panic!("expected lowered call ir"),
        }
    }

    #[test]
    fn lowers_member_access_into_expr_ir_read() {
        let expr = Expr::Member(Box::new(Expr::Ident("user".to_string())), "name".to_string());
        let mut locals = HashMap::new();
        locals.insert("user".to_string(), InferredType::Struct("User".to_string()));
        let mut type_info = TypeInfo::default();
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), InferredType::String);
        type_info.struct_fields.insert("User".to_string(), fields);

        let lowered =
            lower_expr_ir(&expr, &locals, &type_info, &HashSet::new()).expect("expected expr ir");
        match lowered {
            LoweredExprIr::Read(LoweredReadAccess::TypedStructField { field, .. }) => {
                assert_eq!(field, "name");
            }
            _ => panic!("expected lowered read ir"),
        }
    }

    #[test]
    fn lowers_string_concat_into_binary_expr_ir() {
        let expr = Expr::Binary(
            Box::new(Expr::StringLit("a".to_string())),
            "+".to_string(),
            Box::new(Expr::StringLit("b".to_string())),
        );
        let lowered =
            lower_expr_ir(&expr, &HashMap::new(), &TypeInfo::default(), &HashSet::new())
                .expect("expected expr ir");
        match lowered {
            LoweredExprIr::Binary(lowered) => {
                assert_eq!(lowered.result_ty, InferredType::String);
                assert_eq!(lowered.kind, LoweredBinaryExprKind::StringConcat);
            }
            _ => panic!("expected lowered binary ir"),
        }
    }

    #[test]
    fn lowers_boolean_condition_into_expr_ir() {
        let mut locals = HashMap::new();
        locals.insert("ok".to_string(), InferredType::Boolean);
        let lowered = lower_condition_ir(
            &Expr::Ident("ok".to_string()),
            &locals,
            &TypeInfo::default(),
            &HashSet::new(),
        );
        assert_eq!(lowered.kind, ConditionKind::Boolean);
    }
}
