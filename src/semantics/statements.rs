use crate::ast::{Expr, MatchCase, Stmt};
use crate::access::{lower_assignment_target, LoweredAssignmentTarget};
use crate::dispatch::infer_expr_type;
use crate::expressions::{lower_condition_ir, LoweredConditionIr};
use crate::patterns::collect_pattern_binding_types;
use crate::patterns::{lower_destructure, LoweredDestructure};
use crate::typecheck::{InferredType, TypeInfo};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopControlKind {
    Break,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoweredLoopControl {
    pub kind: LoopControlKind,
    pub valid_in_loop: bool,
}

#[derive(Debug, Clone)]
pub struct LoweredMatchCase {
    pub case: MatchCase,
    pub binding_types: HashMap<String, InferredType>,
}

#[derive(Debug, Clone)]
pub struct LoweredMatch {
    pub target_type: InferredType,
    pub cases: Vec<LoweredMatchCase>,
}

#[derive(Debug, Clone)]
pub struct LoweredMatchCaseStmt {
    pub case: MatchCase,
    pub binding_types: HashMap<String, InferredType>,
    pub destructure: LoweredDestructure,
}

#[derive(Debug, Clone)]
pub struct LoweredMatchStmt {
    pub target_type: InferredType,
    pub cases: Vec<LoweredMatchCaseStmt>,
}

#[derive(Debug, Clone)]
pub struct LoweredLetStmt {
    pub name: String,
    pub ty: InferredType,
    pub is_global: bool,
    pub expr: Expr,
}

#[derive(Debug, Clone)]
pub struct LoweredAssignStmt {
    pub target: LoweredAssignmentTarget,
    pub expr: Expr,
}

#[derive(Debug, Clone)]
pub struct LoweredForStmt {
    pub index_name: Option<String>,
    pub item_name: String,
    pub item_ty: InferredType,
    pub iterable: Expr,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum LoweredStmtIr {
    Let {
        line: usize,
        lowered: LoweredLetStmt,
    },
    Assign {
        line: usize,
        lowered: LoweredAssignStmt,
    },
    If {
        line: usize,
        condition: LoweredConditionIr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
        has_else: bool,
    },
    While {
        line: usize,
        condition: LoweredConditionIr,
        body: Vec<Stmt>,
    },
    For {
        line: usize,
        lowered: LoweredForStmt,
    },
    Break {
        line: usize,
        lowered: LoweredLoopControl,
    },
    Continue {
        line: usize,
        lowered: LoweredLoopControl,
    },
    LetPattern {
        line: usize,
        expr: Expr,
        lowered: LoweredDestructure,
    },
    Match {
        line: usize,
        expr: Expr,
        lowered: LoweredMatchStmt,
    },
}

pub fn lower_stmt_ir(
    stmt: &Stmt,
    local_types: &HashMap<String, InferredType>,
    type_info: &TypeInfo,
    globals: &HashSet<String>,
    in_loop: bool,
) -> Option<LoweredStmtIr> {
    match stmt {
        Stmt::Let(line, name, _, expr) => Some(LoweredStmtIr::Let {
            line: *line,
            lowered: LoweredLetStmt {
                name: name.clone(),
                ty: if globals.contains(name) {
                    InferredType::Dynamic
                } else {
                    local_types.get(name).cloned().unwrap_or_else(|| {
                        crate::dispatch::infer_expr_type(expr, local_types, type_info, globals)
                    })
                },
                is_global: globals.contains(name),
                expr: expr.clone(),
            },
        }),
        Stmt::Assign(line, target, expr) => Some(LoweredStmtIr::Assign {
            line: *line,
            lowered: LoweredAssignStmt {
                target: lower_assignment_target(target, local_types, type_info, globals),
                expr: expr.clone(),
            },
        }),
        Stmt::If(line, cond, then_body, else_body) => Some(LoweredStmtIr::If {
            line: *line,
            condition: lower_condition_ir(cond, local_types, type_info, globals),
            then_body: then_body.clone(),
            else_body: else_body.clone(),
            has_else: else_body.is_some(),
        }),
        Stmt::While(line, cond, body) => Some(LoweredStmtIr::While {
            line: *line,
            condition: lower_condition_ir(cond, local_types, type_info, globals),
            body: body.clone(),
        }),
        Stmt::For(line, index_name, item_name, iterable, body) => Some(LoweredStmtIr::For {
            line: *line,
            lowered: {
                let iterable_ty = infer_expr_type(iterable, local_types, type_info, globals);
                match (&index_name, &iterable_ty) {
                    (Some(key_name), InferredType::Map(_value_ty)) => {
                        let entry_name = format!("__map_entry_{}_{}", key_name, item_name);
                        let mut lowered_body = Vec::with_capacity(body.len() + 1);
                        lowered_body.push(Stmt::LetPattern(
                            *line,
                            crate::ast::Pattern::List(
                                vec![
                                    crate::ast::Pattern::Bind(key_name.clone()),
                                    crate::ast::Pattern::Bind(item_name.clone()),
                                ],
                                false,
                            ),
                            Expr::Ident(entry_name.clone()),
                        ));
                        lowered_body.extend(body.clone());
                        LoweredForStmt {
                            index_name: None,
                            item_name: entry_name,
                            item_ty: InferredType::Dynamic,
                            iterable: Expr::MethodCall(
                                Box::new(iterable.clone()),
                                "entries".to_string(),
                                vec![],
                            ),
                            body: lowered_body,
                        }
                    }
                    _ => LoweredForStmt {
                        index_name: index_name.clone(),
                        item_name: item_name.clone(),
                        item_ty: match iterable_ty {
                            InferredType::List(inner) => (*inner).clone(),
                            InferredType::String => InferredType::String,
                            _ => InferredType::Dynamic,
                        },
                        iterable: iterable.clone(),
                        body: body.clone(),
                    },
                }
            },
        }),
        Stmt::Break(line) => Some(LoweredStmtIr::Break {
            line: *line,
            lowered: lower_loop_control(LoopControlKind::Break, in_loop),
        }),
        Stmt::Continue(line) => Some(LoweredStmtIr::Continue {
            line: *line,
            lowered: lower_loop_control(LoopControlKind::Continue, in_loop),
        }),
        Stmt::LetPattern(line, pattern, expr) => Some(LoweredStmtIr::LetPattern {
            line: *line,
            expr: expr.clone(),
            lowered: lower_destructure(
                pattern,
                &crate::dispatch::infer_expr_type(expr, local_types, type_info, globals),
                type_info,
            ),
        }),
        Stmt::Match(line, expr, cases) => Some(LoweredStmtIr::Match {
            line: *line,
            expr: expr.clone(),
            lowered: lower_match_stmt(expr, cases, local_types, type_info, globals),
        }),
        _ => None,
    }
}

pub fn lower_loop_control(kind: LoopControlKind, in_loop: bool) -> LoweredLoopControl {
    LoweredLoopControl {
        kind,
        valid_in_loop: in_loop,
    }
}

pub fn lower_match(
    expr: &Expr,
    cases: &[MatchCase],
    local_types: &HashMap<String, InferredType>,
    type_info: &TypeInfo,
    globals: &HashSet<String>,
) -> LoweredMatch {
    let target_type = infer_expr_type(expr, local_types, type_info, globals);
    let cases = cases
        .iter()
        .cloned()
        .map(|case| {
            let mut binding_types = HashMap::new();
            collect_pattern_binding_types(&case.pattern, &target_type, type_info, &mut binding_types);
            LoweredMatchCase { case, binding_types }
        })
        .collect();
    LoweredMatch { target_type, cases }
}

pub fn lower_match_stmt(
    expr: &Expr,
    cases: &[MatchCase],
    local_types: &HashMap<String, InferredType>,
    type_info: &TypeInfo,
    globals: &HashSet<String>,
) -> LoweredMatchStmt {
    let lowered = lower_match(expr, cases, local_types, type_info, globals);
    lowered_match_stmt(&lowered, type_info)
}

pub fn lowered_match_stmt(
    lowered: &LoweredMatch,
    type_info: &TypeInfo,
) -> LoweredMatchStmt {
    LoweredMatchStmt {
        target_type: lowered.target_type.clone(),
        cases: lowered
            .cases
            .iter()
            .map(|case| LoweredMatchCaseStmt {
                case: case.case.clone(),
                binding_types: case.binding_types.clone(),
                destructure: lower_destructure(&case.case.pattern, &lowered.target_type, type_info),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Pattern;

    #[test]
    fn lowers_match_statement_with_case_destructure() {
        let mut type_info = TypeInfo::default();
        type_info
            .variant_to_enum
            .insert("Some".to_string(), "Option".to_string());
        type_info
            .enum_variant_fields
            .insert("Option.Some".to_string(), vec![InferredType::String]);

        let stmt = Stmt::Match(
            1,
            Expr::Ident("opt".to_string()),
            vec![MatchCase {
                pattern: Pattern::EnumVariant(
                    "Some".to_string(),
                    vec![Pattern::Bind("name".to_string())],
                ),
                guard: None,
                body: vec![],
            }],
        );
        let mut locals = HashMap::new();
        locals.insert("opt".to_string(), InferredType::Enum("Option".to_string()));

        let lowered = lower_stmt_ir(&stmt, &locals, &type_info, &HashSet::new(), false)
            .expect("expected lowered stmt ir");

        match lowered {
            LoweredStmtIr::Match { lowered, .. } => {
                assert_eq!(lowered.target_type, InferredType::Enum("Option".to_string()));
                assert_eq!(lowered.cases.len(), 1);
                assert_eq!(lowered.cases[0].destructure.bindings.len(), 1);
                assert_eq!(lowered.cases[0].destructure.bindings[0].ty, InferredType::String);
            }
            _ => panic!("expected match stmt ir"),
        }
    }

    #[test]
    fn lowers_match_binding_types_for_typed_struct_pattern() {
        let mut type_info = TypeInfo::default();
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), InferredType::String);
        type_info.struct_fields.insert("User".to_string(), fields);

        let cases = vec![MatchCase {
            pattern: crate::ast::Pattern::Struct(
                "User".to_string(),
                vec![("name".to_string(), crate::ast::Pattern::Bind("name".to_string()))],
            ),
            guard: None,
            body: vec![],
        }];

        let mut local_types = HashMap::new();
        local_types.insert("user".to_string(), InferredType::Struct("User".to_string()));

        let lowered = lower_match(
            &Expr::Ident("user".to_string()),
            &cases,
            &local_types,
            &type_info,
            &HashSet::new(),
        );

        assert!(matches!(lowered.target_type, InferredType::Struct(ref name) if name == "User"));
        assert!(matches!(
            lowered.cases[0].binding_types.get("name"),
            Some(InferredType::String)
        ));
    }

    #[test]
    fn lowers_continue_invalidity() {
        let lowered = lower_loop_control(LoopControlKind::Continue, false);
        assert_eq!(lowered.kind, LoopControlKind::Continue);
        assert!(!lowered.valid_in_loop);
    }

    #[test]
    fn lowers_break_statement_with_loop_validity() {
        let lowered = lower_stmt_ir(
            &Stmt::Break(7),
            &HashMap::new(),
            &TypeInfo::default(),
            &HashSet::new(),
            true,
        )
        .expect("expected lowered stmt ir");

        match lowered {
            LoweredStmtIr::Break { line, lowered } => {
                assert_eq!(line, 7);
                assert!(lowered.valid_in_loop);
            }
            _ => panic!("expected break stmt ir"),
        }
    }

    #[test]
    fn lowers_assignment_statement_with_typed_target() {
        let mut locals = HashMap::new();
        locals.insert(
            "items".to_string(),
            InferredType::List(Box::new(InferredType::Number)),
        );
        let lowered = lower_stmt_ir(
            &Stmt::Assign(
                4,
                Expr::Index(
                    Box::new(Expr::Ident("items".to_string())),
                    Box::new(Expr::Number(0.0)),
                ),
                Expr::Number(7.0),
            ),
            &locals,
            &TypeInfo::default(),
            &HashSet::new(),
            false,
        )
        .expect("expected lowered stmt ir");

        match lowered {
            LoweredStmtIr::Assign { line, lowered } => {
                assert_eq!(line, 4);
                assert!(matches!(
                    lowered.target,
                    LoweredAssignmentTarget::TypedListIndex {
                        item_type: InferredType::Number,
                        ..
                    }
                ));
            }
            _ => panic!("expected assign stmt ir"),
        }
    }
}
