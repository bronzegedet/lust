use crate::ast::{Decl, Expr, Pattern, Stmt};
use crate::dispatch::{infer_call_type, infer_method_return_type, rewrite_pipe_expr};
use crate::helpers::{builtin_method_arg_kinds, builtin_method_requires_string_receiver};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferredType {
    Int,
    Number,
    String,
    Boolean,
    List(Box<InferredType>),
    Map(Box<InferredType>),
    Struct(String),
    Enum(String),
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSig {
    pub params: Vec<InferredType>,
    pub ret: InferredType,
}

#[derive(Debug, Clone, Default)]
pub struct TypeInfo {
    pub top_level: HashMap<String, InferredType>,
    pub functions: HashMap<String, HashMap<String, InferredType>>,
    pub signatures: HashMap<String, FunctionSig>,
    pub struct_fields: HashMap<String, HashMap<String, InferredType>>,
    pub enum_variants: HashMap<String, Vec<(String, Vec<String>)>>,
    pub enum_variant_fields: HashMap<String, Vec<InferredType>>,
    pub variant_to_enum: HashMap<String, String>,
}

#[derive(Clone)]
struct FnDeclInfo {
    target: Option<String>,
    params: Vec<(String, Option<String>)>,
    ret_type: Option<String>,
    body: Vec<Stmt>,
}

pub struct TypeChecker {
    decls: HashMap<String, FnDeclInfo>,
    info: TypeInfo,
    errors: Vec<String>,
    explicit_dynamic_struct_fields: HashMap<String, HashSet<String>>,
    explicit_dynamic_params: HashMap<String, Vec<bool>>,
    explicit_dynamic_returns: HashSet<String>,
    explicit_dynamic_locals: HashMap<String, HashSet<String>>,
    active_explicit_dynamic_locals: HashSet<String>,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            decls: HashMap::new(),
            info: TypeInfo::default(),
            errors: Vec::new(),
            explicit_dynamic_struct_fields: HashMap::new(),
            explicit_dynamic_params: HashMap::new(),
            explicit_dynamic_returns: HashSet::new(),
            explicit_dynamic_locals: HashMap::new(),
            active_explicit_dynamic_locals: HashSet::new(),
        }
    }

    pub fn check(mut self, decls: &[Decl]) -> Result<TypeInfo, Vec<String>> {
        self.info.enum_variants.insert(
            "Key".to_string(),
            vec![
                ("Up".to_string(), vec![]),
                ("Down".to_string(), vec![]),
                ("Left".to_string(), vec![]),
                ("Right".to_string(), vec![]),
                ("Enter".to_string(), vec![]),
                ("Esc".to_string(), vec![]),
                ("Char".to_string(), vec!["String".to_string()]),
            ],
        );
        for variant in ["Up", "Down", "Left", "Right", "Enter", "Esc"] {
            self.info.variant_to_enum.insert(variant.to_string(), "Key".to_string());
            self.info
                .enum_variant_fields
                .insert(format!("Key.{}", variant), vec![]);
        }
        self.info.variant_to_enum.insert("Char".to_string(), "Key".to_string());
        self.info
            .enum_variant_fields
            .insert("Key.Char".to_string(), vec![InferredType::String]);

        self.info.enum_variants.insert(
            "FileResult".to_string(),
            vec![
                ("FileOk".to_string(), vec!["String".to_string()]),
                ("FileErr".to_string(), vec!["String".to_string()]),
            ],
        );
        self.info
            .variant_to_enum
            .insert("FileOk".to_string(), "FileResult".to_string());
        self.info
            .enum_variant_fields
            .insert("FileResult.FileOk".to_string(), vec![InferredType::String]);
        self.info
            .variant_to_enum
            .insert("FileErr".to_string(), "FileResult".to_string());
        self.info
            .enum_variant_fields
            .insert("FileResult.FileErr".to_string(), vec![InferredType::String]);

        for decl in decls {
            match decl {
                Decl::Type(name, fields) => {
                    let mut map = HashMap::new();
                    let mut explicit_dynamic = HashSet::new();
                    for (field, declared_ty) in fields {
                        if declared_ty.as_deref() == Some("Dynamic") {
                            explicit_dynamic.insert(field.clone());
                        }
                        map.insert(
                            field.clone(),
                            declared_ty
                                .as_ref()
                                .map(|t| self.declared_type_from_name(t))
                                .unwrap_or(InferredType::Dynamic),
                        );
                    }
                    self.info.struct_fields.insert(name.clone(), map);
                    if !explicit_dynamic.is_empty() {
                        self.explicit_dynamic_struct_fields
                            .insert(name.clone(), explicit_dynamic);
                    }
                }
                Decl::Enum(name, variants) => {
                    self.info.enum_variants.insert(name.clone(), variants.clone());
                    for (variant, fields) in variants {
                        self.info.variant_to_enum.insert(variant.clone(), name.clone());
                        let field_types = fields
                            .iter()
                            .map(|field| self.declared_type_from_name(field))
                            .collect();
                        self.info
                            .enum_variant_fields
                            .insert(format!("{}.{}", name, variant), field_types);
                    }
                }
                Decl::Fn(name, target, params, ret_type, body) => {
                    let key = fn_key(name, target.as_deref());
                    let mut explicit_dynamic_locals = HashSet::new();
                    self.collect_explicit_dynamic_locals(body, &mut explicit_dynamic_locals);
                    if !explicit_dynamic_locals.is_empty() {
                        self.explicit_dynamic_locals
                            .insert(key.clone(), explicit_dynamic_locals);
                    }
                    let explicit_dynamic_params = params
                        .iter()
                        .map(|(_, ty)| ty.as_deref() == Some("Dynamic"))
                        .collect::<Vec<_>>();
                    if explicit_dynamic_params.iter().any(|flag| *flag) {
                        self.explicit_dynamic_params
                            .insert(key.clone(), explicit_dynamic_params);
                    }
                    if ret_type.as_deref() == Some("Dynamic") {
                        self.explicit_dynamic_returns.insert(key.clone());
                    }
                    self.decls.insert(
                        key.clone(),
                        FnDeclInfo {
                            target: target.clone(),
                            params: params.clone(),
                            ret_type: ret_type.clone(),
                            body: body.clone(),
                        },
                    );
                    self.info.signatures.insert(
                        key,
                        FunctionSig {
                            params: params
                                .iter()
                                .map(|(_, ty)| ty.as_ref().map(|t| self.declared_type_from_name(t)).unwrap_or(InferredType::Dynamic))
                                .collect(),
                            ret: ret_type
                                .as_ref()
                                .map(|t| self.declared_type_from_name(t))
                                .unwrap_or(InferredType::Dynamic),
                        },
                    );
                }
                _ => {}
            }
        }

        for _ in 0..8 {
            let prev_sigs = self.info.signatures.clone();
            let prev_fields = self.info.struct_fields.clone();
            self.analyze_top_level(decls);
            self.analyze_functions();
            if self.info.signatures == prev_sigs && self.info.struct_fields == prev_fields {
                break;
            }
        }

        self.validate_matches_in_decls(decls);
        self.validate_loop_control_in_decls(decls);
        self.validate_return_context_in_decls(decls);

        if self.errors.is_empty() {
            Ok(self.info)
        } else {
            Err(self.errors)
        }
    }

    fn analyze_top_level(&mut self, decls: &[Decl]) {
        self.active_explicit_dynamic_locals.clear();
        let mut env = self.info.top_level.clone();
        for decl in decls {
            if let Decl::Stmt(stmt) = decl {
                self.infer_stmt(stmt, &mut env, None);
            }
        }
        self.info.top_level = env;
    }

    fn analyze_functions(&mut self) {
        let keys: Vec<String> = self.decls.keys().cloned().collect();
        for key in keys {
            let decl = match self.decls.get(&key) {
                Some(v) => v.clone(),
                None => continue,
            };
            let sig = self.info.signatures.get(&key).cloned().unwrap_or(FunctionSig {
                params: vec![InferredType::Dynamic; decl.params.len()],
                ret: InferredType::Dynamic,
            });
            self.active_explicit_dynamic_locals = self
                .explicit_dynamic_locals
                .get(&key)
                .cloned()
                .unwrap_or_default();

            let mut env = HashMap::new();
            if let Some(target) = &decl.target {
                env.insert("self".to_string(), InferredType::Struct(target.clone()));
            }
            for ((param, declared_ty), sig_ty) in decl.params.iter().zip(sig.params.iter()) {
                let ty = declared_ty
                    .as_ref()
                    .map(|t| self.declared_type_from_name(t))
                    .unwrap_or_else(|| sig_ty.clone());
                env.insert(param.clone(), ty);
            }

            let mut ret = decl
                .ret_type
                .as_ref()
                .map(|t| self.declared_type_from_name(t))
                .unwrap_or_else(|| sig.ret.clone());
            for stmt in &decl.body {
                self.infer_stmt(stmt, &mut env, Some(&mut ret));
            }

            let param_types = decl
                .params
                .iter()
                .map(|(p, declared_ty)| {
                    declared_ty
                        .as_ref()
                        .map(|t| self.declared_type_from_name(t))
                        .unwrap_or_else(|| env.get(p).cloned().unwrap_or(InferredType::Dynamic))
                })
                .collect();
            self.info.functions.insert(key.clone(), env);
            let keep_dynamic_ret = self.explicit_dynamic_returns.contains(&key);
            self.info.signatures.insert(
                key,
                FunctionSig {
                    params: param_types,
                    ret: if keep_dynamic_ret {
                        InferredType::Dynamic
                    } else {
                        ret
                    },
                },
            );
        }
        self.active_explicit_dynamic_locals.clear();
    }

    fn infer_stmt(
        &mut self,
        stmt: &Stmt,
        env: &mut HashMap<String, InferredType>,
        ret_ty: Option<&mut InferredType>,
    ) {
        match stmt {
            Stmt::Let(line, name, declared_ty, expr) => {
                let expr_ty = self.infer_expr(expr, env);
                let ty = declared_ty
                    .as_ref()
                    .map(|t| self.declared_type_from_name(t))
                    .unwrap_or_else(|| expr_ty.clone());
                if declared_ty.is_some() {
                    self.report_type_mismatch(*line, &ty, &expr_ty, &format!("initializer for '{}'", name));
                }
                env.insert(name.clone(), ty);
            }
            Stmt::LetPattern(_, pattern, expr) => {
                let expr_ty = self.infer_expr(expr, env);
                self.bind_pattern(pattern, &expr_ty, env);
            }
            Stmt::Assign(line, target, expr) => {
                let expr_ty = self.infer_expr(expr, env);
                match target {
                    Expr::Ident(name) => {
                        if self.is_explicit_dynamic_local(name) {
                            env.insert(name.clone(), InferredType::Dynamic);
                            return;
                        }
                        let current = env.get(name).cloned().unwrap_or(InferredType::Dynamic);
                        self.report_type_mismatch(*line, &current, &expr_ty, &format!("assignment to '{}'", name));
                        env.insert(name.clone(), unify(&current, &expr_ty));
                    }
                    Expr::Index(obj, idx) => {
                        match self.infer_expr(obj, env) {
                            InferredType::List(inner) => {
                                constrain_expr(idx, env, InferredType::Number);
                                self.report_type_mismatch(*line, &inner, &expr_ty, "list element assignment");
                                constrain_expr(expr, env, (*inner).clone());
                            }
                            InferredType::Map(inner) => {
                                constrain_expr(idx, env, InferredType::String);
                                self.report_type_mismatch(*line, &inner, &expr_ty, "map value assignment");
                                constrain_expr(expr, env, (*inner).clone());
                            }
                            _ => {}
                        }
                    }
                    Expr::Slice(_, _, _) => {}
                    Expr::Member(obj, field) => {
                        if let InferredType::Struct(name) = self.infer_expr(obj, env) {
                            let expected = self
                                .info
                                .struct_fields
                                .get(&name)
                                .and_then(|m| m.get(field))
                                .cloned()
                                .unwrap_or(InferredType::Dynamic);
                            self.report_type_mismatch(*line, &expected, &expr_ty, &format!("assignment to '{}.{}'", name, field));
                            self.unify_struct_field(&name, field, &expr_ty);
                        }
                    }
                    _ => {}
                }
            }
            Stmt::If(_, cond, then_body, else_body) => {
                self.infer_boolean_expr(cond, env);
                let mut then_env = env.clone();
                for stmt in then_body {
                    self.infer_stmt(stmt, &mut then_env, None);
                }
                let mut merged = then_env;
                if let Some(else_stmts) = else_body {
                    let mut else_env = env.clone();
                    for stmt in else_stmts {
                        self.infer_stmt(stmt, &mut else_env, None);
                    }
                    for (name, else_ty) in else_env {
                        let current = merged.get(&name).cloned().unwrap_or(InferredType::Dynamic);
                        let merged_ty = if self.is_explicit_dynamic_local(&name) {
                            InferredType::Dynamic
                        } else {
                            unify(&current, &else_ty)
                        };
                        merged.insert(name, merged_ty);
                    }
                }
                for (name, ty) in merged {
                    let current = env.get(&name).cloned().unwrap_or(InferredType::Dynamic);
                    let merged_ty = if self.is_explicit_dynamic_local(&name) {
                        InferredType::Dynamic
                    } else {
                        unify(&current, &ty)
                    };
                    env.insert(name, merged_ty);
                }
            }
            Stmt::Match(_, expr, cases) => {
                let match_ty = self.infer_expr(expr, env);
                let mut merged = env.clone();
                for case in cases {
                    let mut case_env = env.clone();
                    self.bind_pattern(&case.pattern, &match_ty, &mut case_env);
                    if let Some(guard) = &case.guard {
                        self.infer_boolean_expr(guard, &mut case_env);
                    }
                    for stmt in &case.body {
                        self.infer_stmt(stmt, &mut case_env, None);
                    }
                    for (name, ty) in case_env {
                        let current = merged.get(&name).cloned().unwrap_or(InferredType::Dynamic);
                        let merged_ty = if self.is_explicit_dynamic_local(&name) {
                            InferredType::Dynamic
                        } else {
                            unify(&current, &ty)
                        };
                        merged.insert(name, merged_ty);
                    }
                }
                *env = merged;
            }
            Stmt::While(_, cond, body) => {
                self.infer_boolean_expr(cond, env);
                let mut body_env = env.clone();
                for stmt in body {
                    self.infer_stmt(stmt, &mut body_env, None);
                }
                for (name, ty) in body_env {
                    let current = env.get(&name).cloned().unwrap_or(InferredType::Dynamic);
                    let merged_ty = if self.is_explicit_dynamic_local(&name) {
                        InferredType::Dynamic
                    } else {
                        unify(&current, &ty)
                    };
                    env.insert(name, merged_ty);
                }
            }
            Stmt::For(_line, index_name, item_name, iterable, body) => {
                let iterable_ty = self.infer_expr(iterable, env);
                let mut body_env = env.clone();
                match iterable_ty {
                    InferredType::List(ref inner) => {
                        if let Some(index_name) = index_name {
                            body_env.insert(index_name.clone(), InferredType::Number);
                        }
                        body_env.insert(item_name.clone(), (**inner).clone());
                    }
                    InferredType::Map(ref inner) => {
                        if let Some(key_name) = index_name {
                            body_env.insert(key_name.clone(), InferredType::String);
                            body_env.insert(item_name.clone(), (**inner).clone());
                        } else {
                            body_env.insert(item_name.clone(), (**inner).clone());
                        }
                    }
                    InferredType::String => {
                        if let Some(index_name) = index_name {
                            body_env.insert(index_name.clone(), InferredType::Number);
                        }
                        body_env.insert(item_name.clone(), InferredType::String);
                    }
                    _ => {
                        if let Some(index_name) = index_name {
                            body_env.insert(index_name.clone(), InferredType::Dynamic);
                        }
                        body_env.insert(item_name.clone(), InferredType::Dynamic);
                    }
                }

                for stmt in body {
                    self.infer_stmt(stmt, &mut body_env, None);
                }
                for (name, ty) in body_env {
                    let current = env.get(&name).cloned().unwrap_or(InferredType::Dynamic);
                    let merged_ty = if self.is_explicit_dynamic_local(&name) {
                        InferredType::Dynamic
                    } else {
                        unify(&current, &ty)
                    };
                    env.insert(name, merged_ty);
                }
            }
            Stmt::Return(line, expr) => {
                let expr_ty = self.infer_expr(expr, env);
                if let Some(ret_ty) = ret_ty {
                    let current = ret_ty.clone();
                    self.report_type_mismatch(*line, &current, &expr_ty, "return value");
                    *ret_ty = unify(&current, &expr_ty);
                }
            }
            Stmt::Pass(_) | Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::ExprStmt(_, expr) => {
                self.infer_expr(expr, env);
            }
            Stmt::Print(_, exprs) | Stmt::Spawn(_, _, exprs) => {
                for expr in exprs {
                    self.infer_expr(expr, env);
                }
            }
        }
    }

    fn infer_boolean_expr(&mut self, expr: &Expr, env: &mut HashMap<String, InferredType>) {
        let ty = self.infer_expr(expr, env);
        if matches!(expr, Expr::Ident(_)) && ty == InferredType::Dynamic {
            constrain_expr(expr, env, InferredType::Boolean);
        }
    }

    fn infer_expr(&mut self, expr: &Expr, env: &mut HashMap<String, InferredType>) -> InferredType {
        match expr {
            Expr::Number(value) => {
                if is_integral_number(*value) {
                    InferredType::Int
                } else {
                    InferredType::Number
                }
            }
            Expr::StringLit(_) => InferredType::String,
            Expr::Ident(name) => env.get(name).cloned().unwrap_or(InferredType::Dynamic),
            Expr::Self_ => env.get("self").cloned().unwrap_or(InferredType::Dynamic),
            Expr::Lambda(_, _) => InferredType::Dynamic,
            Expr::Pipe(target, name, args) => {
                let rewritten = self.rewrite_pipe_expr(target, name, args, env);
                self.infer_expr(&rewritten, env)
            }
            Expr::List(items) => {
                let mut elem_ty = InferredType::Dynamic;
                for item in items {
                    let item_ty = self.infer_expr(item, env);
                    elem_ty = unify(&elem_ty, &item_ty);
                }
                InferredType::List(Box::new(elem_ty))
            }
            Expr::MapLit(items) => {
                let mut value_ty = InferredType::Dynamic;
                for (key, value) in items {
                    constrain_expr(key, env, InferredType::String);
                    self.infer_expr(key, env);
                    let item_ty = self.infer_expr(value, env);
                    value_ty = unify(&value_ty, &item_ty);
                }
                InferredType::Map(Box::new(value_ty))
            }
            Expr::Index(target, index) => {
                let target_ty = self.infer_expr(target, env);
                match target_ty {
                    InferredType::List(inner) => {
                        constrain_expr(index, env, InferredType::Number);
                        (*inner).clone()
                    }
                    InferredType::Map(inner) => {
                        constrain_expr(index, env, InferredType::String);
                        (*inner).clone()
                    }
                    InferredType::String => {
                        constrain_expr(index, env, InferredType::Number);
                        InferredType::String
                    }
                    _ => InferredType::Dynamic,
                }
            }
            Expr::Slice(target, start, end) => {
                let target_ty = self.infer_expr(target, env);
                if let Some(start) = start {
                    constrain_expr(start, env, InferredType::Number);
                }
                if let Some(end) = end {
                    constrain_expr(end, env, InferredType::Number);
                }
                match target_ty {
                    InferredType::List(inner) => InferredType::List(inner),
                    InferredType::String => InferredType::String,
                    _ => InferredType::Dynamic,
                }
            }
            Expr::StructInst(name, fields, base) => {
                if let Some(base_expr) = base {
                    let base_ty = self.infer_expr(base_expr, env);
                    self.report_type_mismatch(
                        0,
                        &InferredType::Struct(name.clone()),
                        &base_ty,
                        &format!("struct update base for '{}'", name),
                    );
                }
                for (field, value) in fields {
                    let val_ty = self.infer_expr(value, env);
                    let expected = self
                        .info
                        .struct_fields
                        .get(name)
                        .and_then(|m| m.get(field))
                        .cloned()
                        .unwrap_or(InferredType::Dynamic);
                    self.report_type_mismatch(0, &expected, &val_ty, &format!("field '{}.{}'", name, field));
                    self.unify_struct_field(name, field, &val_ty);
                }
                InferredType::Struct(name.clone())
            }
            Expr::EnumVariant(name, args) => {
                if let Some(enum_name) = self.info.variant_to_enum.get(name).cloned() {
                    let key = format!("{}.{}", enum_name, name);
                    if let Some(field_tys) = self.info.enum_variant_fields.get(&key).cloned() {
                        for (idx, arg) in args.iter().enumerate() {
                            let arg_ty = self.infer_expr(arg, env);
                            if let Some(expected) = field_tys.get(idx) {
                                self.report_type_mismatch(0, expected, &arg_ty, &format!("payload {} for enum variant '{}'", idx + 1, name));
                                constrain_expr(arg, env, expected.clone());
                            }
                            self.unify_enum_variant_field(&enum_name, name, idx, &arg_ty);
                        }
                    } else {
                        for arg in args {
                            self.infer_expr(arg, env);
                        }
                    }
                    InferredType::Enum(enum_name)
                } else {
                    InferredType::Dynamic
                }
            }
            Expr::Member(target, field) => {
                if let Expr::Ident(enum_name) = target.as_ref() {
                    if self
                        .info
                        .variant_to_enum
                        .get(field)
                        .map(|owner| owner == enum_name)
                        .unwrap_or(false)
                    {
                        return InferredType::Enum(enum_name.clone());
                    }
                }
                if let InferredType::Struct(name) = self.infer_expr(target, env) {
                    return self
                        .info
                        .struct_fields
                        .get(&name)
                        .and_then(|m| m.get(field))
                        .cloned()
                        .unwrap_or(InferredType::Dynamic);
                }
                InferredType::Dynamic
            }
            Expr::MethodCall(target, name, args) => {
                let target_ty = self.infer_expr(target, env);
                if builtin_method_requires_string_receiver(name) {
                    constrain_expr(target, env, InferredType::String);
                }
                if name == "push" {
                    if let Some(arg) = args.first() {
                        let arg_ty = self.infer_expr(arg, env);
                        match &target_ty {
                            InferredType::List(inner) => {
                                constrain_expr(arg, env, (**inner).clone());
                            }
                            InferredType::Dynamic => {
                                if let Expr::Ident(name) = target.as_ref() {
                                    if !self.is_explicit_dynamic_local(name)
                                        && arg_ty != InferredType::Dynamic
                                    {
                                        let list_ty = InferredType::List(Box::new(arg_ty.clone()));
                                        let current = env.get(name).cloned().unwrap_or(InferredType::Dynamic);
                                        env.insert(name.clone(), unify(&current, &list_ty));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                for arg in args {
                    self.infer_expr(arg, env);
                }
                if let InferredType::Struct(struct_name) = &target_ty {
                    let key = format!("{}.{}", struct_name, name);
                    if let Some(sig) = self.info.signatures.get(&key).cloned() {
                        self.report_arity_mismatch(0, sig.params.len(), args.len(), &format!("call to '{}.{}'", struct_name, name));
                        for (idx, arg) in args.iter().enumerate() {
                            if let Some(param_ty) = sig.params.get(idx) {
                                let arg_ty = self.infer_expr(arg, env);
                                self.report_type_mismatch(0, param_ty, &arg_ty, &format!("argument {} in call to '{}.{}'", idx + 1, struct_name, name));
                                constrain_expr(arg, env, param_ty.clone());
                                self.unify_fn_param(&key, idx, &arg_ty);
                            }
                        }
                        return sig.ret;
                    }
                }
                if let Some(arg_kinds) = builtin_method_arg_kinds(name) {
                    for (arg, kind) in args.iter().zip(arg_kinds.iter()) {
                        match kind {
                            crate::helpers::BuiltinArgKind::String => {
                                constrain_expr(arg, env, InferredType::String);
                            }
                            crate::helpers::BuiltinArgKind::Number => {
                                constrain_expr(arg, env, InferredType::Number);
                            }
                            crate::helpers::BuiltinArgKind::Any => {}
                        }
                    }
                }
                infer_method_return_type(&target_ty, name, &self.info)
            }
            Expr::Call(name, args) => {
                if name == "dict" {
                    return self.infer_dict_expr(args, env);
                }
                if name == "filter" || name == "map" {
                    return self.infer_list_transform(name, args, env);
                }
                if name == "map_values" || name == "filter_values" || name == "map_entries" || name == "filter_entries" {
                    return self.infer_map_transform(name, args, env);
                }
                let arg_types: Vec<InferredType> = args.iter().map(|a| self.infer_expr(a, env)).collect();
                match name.as_str() {
                    "sin" | "cos" | "sqrt" | "abs" | "random_int" | "random_float" | "sleep" => {
                        for arg in args {
                            constrain_expr(arg, env, InferredType::Number);
                        }
                    }
                    "panic" => {
                        for arg in args {
                            constrain_expr(arg, env, InferredType::String);
                        }
                    }
                    "type_of" => {}
                    "assert" => {
                        if let Some(cond) = args.first() {
                            constrain_expr(cond, env, InferredType::Boolean);
                        }
                        if let Some(message) = args.get(1) {
                            constrain_expr(message, env, InferredType::String);
                        }
                    }
                    "debug" => {
                        if let Some(label) = args.first() {
                            constrain_expr(label, env, InferredType::String);
                        }
                    }
                    "get_env" | "json_decode" | "json_parse" | "compile_lustgex" | "read_file" | "read_file_result" | "try_read_file" | "write_file" | "append_file" | "launch_lust" | "open_file" | "list_dir" => {
                        for arg in args {
                            constrain_expr(arg, env, InferredType::String);
                        }
                    }
                    "__str_insert" => {
                        if let Some(text) = args.first() {
                            constrain_expr(text, env, InferredType::String);
                        }
                        if let Some(index) = args.get(1) {
                            constrain_expr(index, env, InferredType::Number);
                        }
                        if let Some(part) = args.get(2) {
                            constrain_expr(part, env, InferredType::String);
                        }
                    }
                    "__str_delete_range" => {
                        if let Some(text) = args.first() {
                            constrain_expr(text, env, InferredType::String);
                        }
                        if let Some(start) = args.get(1) {
                            constrain_expr(start, env, InferredType::Number);
                        }
                        if let Some(end) = args.get(2) {
                            constrain_expr(end, env, InferredType::Number);
                        }
                    }
                    "__str_find" => {
                        if let Some(text) = args.first() {
                            constrain_expr(text, env, InferredType::String);
                        }
                        if let Some(needle) = args.get(1) {
                            constrain_expr(needle, env, InferredType::String);
                        }
                    }
                    "ui_knob" | "ui_slider" => {
                        if let Some(id) = args.first() {
                            constrain_expr(id, env, InferredType::String);
                        }
                        if let Some(min) = args.get(1) {
                            constrain_expr(min, env, InferredType::Number);
                        }
                        if let Some(max) = args.get(2) {
                            constrain_expr(max, env, InferredType::Number);
                        }
                        if let Some(default_value) = args.get(3) {
                            constrain_expr(default_value, env, InferredType::Number);
                        }
                    }
                    "ui_toggle" => {
                        if let Some(id) = args.first() {
                            constrain_expr(id, env, InferredType::String);
                        }
                        if let Some(default_value) = args.get(1) {
                            constrain_expr(default_value, env, InferredType::Boolean);
                        }
                    }
                    "ui_textbox" => {
                        if let Some(id) = args.first() {
                            constrain_expr(id, env, InferredType::String);
                        }
                        if let Some(default_value) = args.get(1) {
                            constrain_expr(default_value, env, InferredType::String);
                        }
                    }
                    "ui_caret" | "ui_selection_start" | "ui_selection_end" | "ui_scroll_y" => {
                        if let Some(id) = args.first() {
                            constrain_expr(id, env, InferredType::String);
                        }
                        if let Some(default_value) = args.get(1) {
                            constrain_expr(default_value, env, InferredType::Number);
                        }
                    }
                    "ui_text_input" => {}
                    "ui_key_left"
                    | "ui_key_right"
                    | "ui_key_up"
                    | "ui_key_down"
                    | "ui_key_enter"
                    | "ui_key_esc"
                    | "ui_key_backspace"
                    | "ui_key_delete" => {}
                    "ui_mouse_x"
                    | "ui_mouse_y"
                    | "ui_mouse_down"
                    | "ui_mouse_clicked"
                    | "ui_mouse_click_x"
                    | "ui_mouse_click_y" => {}
                    "ui_command" => {
                        if let Some(id) = args.first() {
                            constrain_expr(id, env, InferredType::String);
                        }
                        if let Some(default_value) = args.get(1) {
                            constrain_expr(default_value, env, InferredType::String);
                        }
                    }
                    "ui_button" => {
                        if let Some(id) = args.first() {
                            constrain_expr(id, env, InferredType::String);
                        }
                    }
                    "ui_theme" => {
                        if let Some(theme_name) = args.first() {
                            constrain_expr(theme_name, env, InferredType::String);
                        }
                    }
                    "ui_set" | "ui_get" => {
                        if let Some(id) = args.first() {
                            constrain_expr(id, env, InferredType::String);
                        }
                    }
                    "json_encode" => {}
                    "lustgex_match" => {
                        for arg in args {
                            constrain_expr(arg, env, InferredType::String);
                        }
                    }
                    "__range" | "__range_inclusive" => {
                        for arg in args {
                            constrain_expr(arg, env, InferredType::Number);
                        }
                    }
                    "clr" => {}
                    "prompt" => {
                        if let Some(message) = args.first() {
                            constrain_expr(message, env, InferredType::String);
                        }
                    }
                    "lustgex_capture_builtin" => {
                        for arg in args {
                            constrain_expr(arg, env, InferredType::String);
                        }
                    }
                    "regex_capture" => {
                        if let Some(text) = args.first() {
                            constrain_expr(text, env, InferredType::String);
                        }
                        if let Some(pattern) = args.get(1) {
                            constrain_expr(pattern, env, InferredType::String);
                        }
                    }
                    "to_string" => {}
                    _ => {
                        if let Some(sig) = self.info.signatures.get(name).cloned() {
                            self.report_arity_mismatch(0, sig.params.len(), args.len(), &format!("call to '{}'", name));
                            for (idx, arg) in args.iter().enumerate() {
                                if let Some(param_ty) = sig.params.get(idx) {
                                    self.report_type_mismatch(0, param_ty, &arg_types[idx], &format!("argument {} in call to '{}'", idx + 1, name));
                                    constrain_expr(arg, env, param_ty.clone());
                                    self.unify_fn_param(name, idx, &arg_types[idx]);
                                }
                            }
                            return sig.ret;
                        }
                    }
                }
                infer_call_type(name)
            }
            Expr::Binary(left, op, right) => {
                let left_ty = self.infer_expr(left, env);
                let right_ty = self.infer_expr(right, env);
                match op.as_str() {
                    "+" => {
                        if left_ty == InferredType::String || right_ty == InferredType::String {
                            constrain_expr(left, env, InferredType::String);
                            constrain_expr(right, env, InferredType::String);
                            InferredType::String
                        } else if is_numeric_type(&left_ty) && is_numeric_type(&right_ty) {
                            constrain_expr(left, env, InferredType::Number);
                            constrain_expr(right, env, InferredType::Number);
                            if left_ty == InferredType::Int && right_ty == InferredType::Int {
                                InferredType::Int
                            } else {
                                InferredType::Number
                            }
                        } else {
                            InferredType::Dynamic
                        }
                    }
                    "-" | "*" | "%" => {
                        constrain_expr(left, env, InferredType::Number);
                        constrain_expr(right, env, InferredType::Number);
                        if left_ty == InferredType::Int && right_ty == InferredType::Int {
                            InferredType::Int
                        } else {
                            InferredType::Number
                        }
                    }
                    "/" => {
                        constrain_expr(left, env, InferredType::Number);
                        constrain_expr(right, env, InferredType::Number);
                        InferredType::Number
                    }
                    "==" | "!=" | ">" | "<" | ">=" | "<=" => {
                        if left_ty != InferredType::Dynamic {
                            constrain_expr(right, env, left_ty.clone());
                        } else if right_ty != InferredType::Dynamic {
                            constrain_expr(left, env, right_ty.clone());
                        }
                        InferredType::Boolean
                    }
                    "and" | "or" => {
                        constrain_expr(left, env, InferredType::Boolean);
                        constrain_expr(right, env, InferredType::Boolean);
                        InferredType::Boolean
                    }
                    "not" => {
                        constrain_expr(right, env, InferredType::Boolean);
                        InferredType::Boolean
                    }
                    _ => InferredType::Dynamic,
                }
            }
        }
    }

    fn rewrite_pipe_expr(
        &mut self,
        target: &Expr,
        name: &str,
        args: &[Expr],
        env: &mut HashMap<String, InferredType>,
    ) -> Expr {
        let target_ty = self.infer_expr(target, env);
        rewrite_pipe_expr(target, name, args, &target_ty, &self.info)
    }

    fn infer_list_transform(
        &mut self,
        name: &str,
        args: &[Expr],
        env: &mut HashMap<String, InferredType>,
    ) -> InferredType {
        if args.len() < 2 {
            for arg in args {
                self.infer_expr(arg, env);
            }
            return InferredType::Dynamic;
        }

        let source_ty = self.infer_expr(&args[0], env);
        let InferredType::List(inner) = source_ty.clone() else {
            self.infer_expr(&args[1], env);
            return InferredType::Dynamic;
        };
        let Expr::Lambda(params, body) = &args[1] else {
            self.infer_expr(&args[1], env);
            return InferredType::Dynamic;
        };
        if params.len() != 1 {
            return InferredType::Dynamic;
        }

        let mut lambda_env = env.clone();
        lambda_env.insert(params[0].clone(), (*inner).clone());
        let body_ty = self.infer_expr(body, &mut lambda_env);
        if name == "filter" {
            constrain_expr(body, &mut lambda_env, InferredType::Boolean);
            return InferredType::List(inner);
        }
        match body_ty {
            InferredType::Dynamic => InferredType::Dynamic,
            _ => InferredType::List(Box::new(body_ty)),
        }
    }

    fn infer_dict_expr(
        &mut self,
        args: &[Expr],
        env: &mut HashMap<String, InferredType>,
    ) -> InferredType {
        let mut value_ty = InferredType::Dynamic;
        for (idx, arg) in args.iter().enumerate() {
            if idx % 2 == 0 {
                constrain_expr(arg, env, InferredType::String);
                self.infer_expr(arg, env);
            } else {
                let arg_ty = self.infer_expr(arg, env);
                value_ty = unify(&value_ty, &arg_ty);
            }
        }
        InferredType::Map(Box::new(value_ty))
    }

    fn infer_map_transform(
        &mut self,
        name: &str,
        args: &[Expr],
        env: &mut HashMap<String, InferredType>,
    ) -> InferredType {
        if args.len() < 2 {
            for arg in args {
                self.infer_expr(arg, env);
            }
            return InferredType::Dynamic;
        }

        let source_ty = self.infer_expr(&args[0], env);
        let InferredType::Map(inner) = source_ty.clone() else {
            self.infer_expr(&args[1], env);
            return InferredType::Dynamic;
        };
        let Expr::Lambda(params, body) = &args[1] else {
            self.infer_expr(&args[1], env);
            return InferredType::Dynamic;
        };
        if params.len() != 1 {
            return InferredType::Dynamic;
        }

        let mut lambda_env = env.clone();
        if name == "map_entries" || name == "filter_entries" {
            lambda_env.insert(
                params[0].clone(),
                InferredType::List(Box::new(InferredType::Dynamic)),
            );
        } else {
            lambda_env.insert(params[0].clone(), (*inner).clone());
        }
        let body_ty = self.infer_expr(body, &mut lambda_env);
        if name == "filter_values" || name == "filter_entries" {
            constrain_expr(body, &mut lambda_env, InferredType::Boolean);
            return InferredType::Map(inner);
        }
        if name == "map_entries" {
            return InferredType::Map(Box::new(InferredType::Dynamic));
        }
        match body_ty {
            InferredType::Dynamic => InferredType::Dynamic,
            _ => InferredType::Map(Box::new(body_ty)),
        }
    }

    fn unify_fn_param(&mut self, name: &str, idx: usize, arg_ty: &InferredType) {
        if self
            .explicit_dynamic_params
            .get(name)
            .and_then(|flags| flags.get(idx))
            .copied()
            .unwrap_or(false)
        {
            return;
        }
        if let Some(sig) = self.info.signatures.get_mut(name) {
            if let Some(param) = sig.params.get_mut(idx) {
                *param = if *param == InferredType::Dynamic {
                    unify(param, arg_ty)
                } else {
                    param.clone()
                };
            }
        }
    }

    fn unify_struct_field(&mut self, struct_name: &str, field: &str, value_ty: &InferredType) {
        if self
            .explicit_dynamic_struct_fields
            .get(struct_name)
            .map(|fields| fields.contains(field))
            .unwrap_or(false)
        {
            return;
        }
        if let Some(fields) = self.info.struct_fields.get_mut(struct_name) {
            let current = fields.get(field).cloned().unwrap_or(InferredType::Dynamic);
            fields.insert(
                field.to_string(),
                if current == InferredType::Dynamic {
                    unify(&current, value_ty)
                } else {
                    current
                },
            );
        }
    }

    fn unify_enum_variant_field(&mut self, enum_name: &str, variant: &str, idx: usize, value_ty: &InferredType) {
        if let Some(fields) = self.info.enum_variant_fields.get_mut(&format!("{}.{}", enum_name, variant)) {
            if let Some(current) = fields.get(idx).cloned() {
                fields[idx] = if current == InferredType::Dynamic {
                    unify(&current, value_ty)
                } else {
                    current
                };
            }
        }
    }

    fn declared_type_from_name(&self, name: &str) -> InferredType {
        match name {
            "Number" => InferredType::Number,
            "Int" => InferredType::Int,
            "String" => InferredType::String,
            "Boolean" => InferredType::Boolean,
            _ if name.starts_with("List<") && name.ends_with('>') => {
                let inner = &name[5..name.len() - 1];
                InferredType::List(Box::new(self.declared_type_from_name(inner)))
            }
            _ if name.starts_with("Map<") && name.ends_with('>') => {
                let inner = &name[4..name.len() - 1];
                InferredType::Map(Box::new(self.declared_type_from_name(inner)))
            }
            _ if self.info.struct_fields.contains_key(name) => InferredType::Struct(name.to_string()),
            _ if self.info.enum_variants.contains_key(name) => InferredType::Enum(name.to_string()),
            _ => InferredType::Dynamic,
        }
    }

    fn report_type_mismatch(&mut self, line: usize, expected: &InferredType, actual: &InferredType, context: &str) {
        if !types_compatible(expected, actual) {
            let prefix = if line == 0 {
                "Lust Error".to_string()
            } else {
                format!("Lust Error on line {}", line)
            };
            self.errors.push(format!(
                "{}: type mismatch in {}: expected {}, got {}",
                prefix,
                context,
                type_name(expected),
                type_name(actual)
            ));
        }
    }

    fn report_arity_mismatch(&mut self, line: usize, expected: usize, actual: usize, context: &str) {
        if expected != actual {
            let prefix = if line == 0 {
                "Lust Error".to_string()
            } else {
                format!("Lust Error on line {}", line)
            };
            self.errors.push(format!(
                "{}: arity mismatch in {}: expected {}, got {}",
                prefix, context, expected, actual
            ));
        }
    }

    fn bind_pattern(&mut self, pattern: &Pattern, target_ty: &InferredType, env: &mut HashMap<String, InferredType>) {
        match pattern {
            Pattern::Wildcard | Pattern::Number(_) | Pattern::StringLit(_) | Pattern::Bool(_) | Pattern::Null => {}
            Pattern::Bind(name) => {
                env.insert(name.clone(), target_ty.clone());
            }
            Pattern::List(parts, _) => {
                let item_ty = match target_ty {
                    InferredType::List(inner) => (**inner).clone(),
                    _ => InferredType::Dynamic,
                };
                for pat in parts {
                    self.bind_pattern(pat, &item_ty, env);
                }
            }
            Pattern::Struct(name, fields) => {
                if let InferredType::Struct(struct_name) = target_ty {
                    if name == struct_name {
                        for (field, pat) in fields {
                            let field_ty = self
                                .info
                                .struct_fields
                                .get(struct_name)
                                .and_then(|m| m.get(field))
                                .cloned()
                                .unwrap_or(InferredType::Dynamic);
                            self.bind_pattern(pat, &field_ty, env);
                        }
                    }
                }
            }
            Pattern::EnumVariant(name, parts) => {
                if let InferredType::Enum(enum_name) = target_ty {
                    if let Some(owner) = self.info.variant_to_enum.get(name) {
                        if owner == enum_name {
                            if let Some(field_tys) = self.info.enum_variant_fields.get(&format!("{}.{}", enum_name, name)).cloned() {
                                for (idx, pat) in parts.iter().enumerate() {
                                    let field_ty = field_tys.get(idx).cloned().unwrap_or(InferredType::Dynamic);
                                    self.bind_pattern(pat, &field_ty, env);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn validate_matches_in_decls(&mut self, decls: &[Decl]) {
        for decl in decls {
            match decl {
                Decl::Fn(_, _, params, _, body) => {
                    let mut env = HashMap::new();
                    for (name, declared_ty) in params {
                        let ty = declared_ty
                            .as_deref()
                            .map(|t| self.declared_type_from_name(t))
                            .unwrap_or(InferredType::Dynamic);
                        env.insert(name.clone(), ty);
                    }
                    self.validate_matches_in_stmts(body, &mut env);
                }
                Decl::Stmt(stmt) => {
                    let mut env = self.info.top_level.clone();
                    self.validate_matches_in_stmt(stmt, &mut env);
                }
                _ => {}
            }
        }
    }

    fn validate_matches_in_stmts(&mut self, stmts: &[Stmt], env: &mut HashMap<String, InferredType>) {
        for stmt in stmts {
            self.validate_matches_in_stmt(stmt, env);
        }
    }

    fn validate_matches_in_stmt(&mut self, stmt: &Stmt, env: &mut HashMap<String, InferredType>) {
        match stmt {
            Stmt::Let(_, name, declared_ty, expr) => {
                let ty = declared_ty
                    .as_deref()
                    .map(|t| self.declared_type_from_name(t))
                    .unwrap_or_else(|| self.resolve_expr_type_for_validation(expr, env));
                env.insert(name.clone(), ty);
            }
            Stmt::If(_, _, then_body, else_body) => {
                let mut then_env = env.clone();
                self.validate_matches_in_stmts(then_body, &mut then_env);
                if let Some(else_body) = else_body {
                    let mut else_env = env.clone();
                    self.validate_matches_in_stmts(else_body, &mut else_env);
                }
            }
            Stmt::Match(_, expr, cases) => {
                let match_ty = self.resolve_expr_type_for_validation(expr, env);
                self.validate_match_exhaustiveness(stmt, &match_ty, cases);
                for case in cases {
                    let mut case_env = env.clone();
                    self.validate_matches_in_stmts(&case.body, &mut case_env);
                }
            }
            Stmt::While(_, _, body) => {
                let mut body_env = env.clone();
                self.validate_matches_in_stmts(body, &mut body_env);
            }
            Stmt::For(_, _, _, _, body) => {
                let mut body_env = env.clone();
                self.validate_matches_in_stmts(body, &mut body_env);
            }
            _ => {}
        }
    }

    fn validate_loop_control_in_decls(&mut self, decls: &[Decl]) {
        for decl in decls {
            match decl {
                Decl::Fn(_, _, _, _, body) => self.validate_loop_control_in_stmts(body, 0),
                Decl::Stmt(stmt) => self.validate_loop_control_in_stmt(stmt, 0),
                _ => {}
            }
        }
    }

    fn validate_loop_control_in_stmts(&mut self, stmts: &[Stmt], loop_depth: usize) {
        for stmt in stmts {
            self.validate_loop_control_in_stmt(stmt, loop_depth);
        }
    }

    fn validate_loop_control_in_stmt(&mut self, stmt: &Stmt, loop_depth: usize) {
        match stmt {
            Stmt::Break(line) => {
                if loop_depth == 0 {
                    self.errors.push(format!(
                        "Lust Error on line {}: break is only valid inside a while loop",
                        line
                    ));
                }
            }
            Stmt::Continue(line) => {
                if loop_depth == 0 {
                    self.errors.push(format!(
                        "Lust Error on line {}: continue is only valid inside a while loop",
                        line
                    ));
                }
            }
            Stmt::If(_, _, then_body, else_body) => {
                self.validate_loop_control_in_stmts(then_body, loop_depth);
                if let Some(else_body) = else_body {
                    self.validate_loop_control_in_stmts(else_body, loop_depth);
                }
            }
            Stmt::Match(_, _, cases) => {
                for case in cases {
                    self.validate_loop_control_in_stmts(&case.body, loop_depth);
                }
            }
            Stmt::While(_, _, body) => self.validate_loop_control_in_stmts(body, loop_depth + 1),
            Stmt::For(_, _, _, _, body) => self.validate_loop_control_in_stmts(body, loop_depth + 1),
            _ => {}
        }
    }

    fn validate_return_context_in_decls(&mut self, decls: &[Decl]) {
        for decl in decls {
            match decl {
                Decl::Fn(_, _, _, _, body) => self.validate_return_context_in_stmts(body, true),
                Decl::Stmt(stmt) => self.validate_return_context_in_stmt(stmt, false),
                _ => {}
            }
        }
    }

    fn validate_return_context_in_stmts(&mut self, stmts: &[Stmt], in_function: bool) {
        for stmt in stmts {
            self.validate_return_context_in_stmt(stmt, in_function);
        }
    }

    fn validate_return_context_in_stmt(&mut self, stmt: &Stmt, in_function: bool) {
        match stmt {
            Stmt::Return(line, _) => {
                if !in_function {
                    self.errors.push(format!(
                        "Lust Error on line {}: return is only valid inside a function",
                        line
                    ));
                }
            }
            Stmt::If(_, _, then_body, else_body) => {
                self.validate_return_context_in_stmts(then_body, in_function);
                if let Some(else_body) = else_body {
                    self.validate_return_context_in_stmts(else_body, in_function);
                }
            }
            Stmt::Match(_, _, cases) => {
                for case in cases {
                    self.validate_return_context_in_stmts(&case.body, in_function);
                }
            }
            Stmt::While(_, _, body) => self.validate_return_context_in_stmts(body, in_function),
            Stmt::For(_, _, _, _, body) => self.validate_return_context_in_stmts(body, in_function),
            _ => {}
        }
    }

    fn validate_match_exhaustiveness(
        &mut self,
        stmt: &Stmt,
        match_ty: &InferredType,
        cases: &[crate::ast::MatchCase],
    ) {
        let line = stmt.line();
        let mut has_total_coverage = false;
        let mut seen_unconditional_total = false;
        let mut seen_variants = Vec::new();
        let mut inferred_enum_name = None;
        let mut enum_cases_are_consistent = true;
        for case in cases {
            if seen_unconditional_total {
                self.errors.push(format!(
                    "Lust Error on line {}: unreachable match case after a total case",
                    line
                ));
                break;
            }
            let is_total_pattern = self.pattern_is_total_for_type(&case.pattern, match_ty);
            if !matches!(match_ty, InferredType::Enum(_)) && is_total_pattern {
                has_total_coverage = true;
                if case.guard.is_none() {
                    seen_unconditional_total = true;
                }
            }
            if case.guard.is_none() {
                match &case.pattern {
                    Pattern::EnumVariant(name, _) => {
                        if let Some(owner) = self.info.variant_to_enum.get(name) {
                            match inferred_enum_name.as_ref() {
                                Some(existing) if existing != owner => {
                                    enum_cases_are_consistent = false;
                                }
                                None => inferred_enum_name = Some(owner.clone()),
                                _ => {}
                            }
                        } else {
                            enum_cases_are_consistent = false;
                        }
                        seen_variants.push(name.clone());
                    }
                    pat if self.pattern_is_total_for_type(pat, match_ty) => {
                        has_total_coverage = true;
                        seen_unconditional_total = true;
                    }
                    _ => {}
                }
            }
        }

        let enum_name = match match_ty {
            InferredType::Enum(enum_name) => Some(enum_name.clone()),
            _ if enum_cases_are_consistent => inferred_enum_name,
            _ => None,
        };

        match enum_name {
            Some(enum_name) if !has_total_coverage => {
                let expected = self
                    .info
                    .enum_variants
                    .get(&enum_name)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>();
                let all_seen = expected.iter().all(|name| seen_variants.contains(name));
                if !all_seen {
                    self.errors.push(format!(
                        "Lust Error on line {}: non-exhaustive match on enum {}; missing variants or `_` fallback",
                        line, enum_name
                    ));
                }
            }
            None if !has_total_coverage => {
                self.errors.push(format!(
                    "Lust Error on line {}: non-exhaustive match; add `_` or an unguarded binding case",
                    line
                ));
            }
            _ => {}
        }
    }

    fn resolve_expr_type_for_validation(&self, expr: &Expr, env: &HashMap<String, InferredType>) -> InferredType {
        match expr {
            Expr::Ident(name) => env.get(name).cloned().or_else(|| self.info.top_level.get(name).cloned()).unwrap_or(InferredType::Dynamic),
            Expr::StructInst(name, _, _) => InferredType::Struct(name.clone()),
            Expr::Member(target, field) => {
                if let Expr::Ident(enum_name) = target.as_ref() {
                    if self
                        .info
                        .variant_to_enum
                        .get(field)
                        .map(|owner| owner == enum_name)
                        .unwrap_or(false)
                    {
                        return InferredType::Enum(enum_name.clone());
                    }
                }
                InferredType::Dynamic
            }
            Expr::EnumVariant(name, _) => self
                .info
                .variant_to_enum
                .get(name)
                .cloned()
                .map(InferredType::Enum)
                .unwrap_or(InferredType::Dynamic),
            _ => InferredType::Dynamic,
        }
    }

    fn is_explicit_dynamic_local(&self, name: &str) -> bool {
        self.active_explicit_dynamic_locals.contains(name)
    }

    fn pattern_is_total_for_type(&self, pattern: &Pattern, match_ty: &InferredType) -> bool {
        match pattern {
            Pattern::Wildcard | Pattern::Bind(_) => true,
            Pattern::Struct(name, fields) => {
                if !matches!(match_ty, InferredType::Struct(expected) if expected == name) {
                    return false;
                }
                let Some(field_types) = self.info.struct_fields.get(name) else {
                    return false;
                };
                fields.iter().all(|(field, subpattern)| {
                    let field_ty = field_types.get(field).cloned().unwrap_or(InferredType::Dynamic);
                    self.pattern_is_total_for_type(subpattern, &field_ty)
                })
            }
            _ => false,
        }
    }

    fn collect_explicit_dynamic_locals(&self, body: &[Stmt], out: &mut HashSet<String>) {
        for stmt in body {
            match stmt {
                Stmt::Let(_, name, declared_ty, _) if declared_ty.as_deref() == Some("Dynamic") => {
                    out.insert(name.clone());
                }
                Stmt::If(_, _, then_body, else_body) => {
                    self.collect_explicit_dynamic_locals(then_body, out);
                    if let Some(else_body) = else_body {
                        self.collect_explicit_dynamic_locals(else_body, out);
                    }
                }
                Stmt::Match(_, _, cases) => {
                    for case in cases {
                        self.collect_explicit_dynamic_locals(&case.body, out);
                    }
                }
                Stmt::While(_, _, body) => self.collect_explicit_dynamic_locals(body, out),
                Stmt::For(_, _, _, _, body) => self.collect_explicit_dynamic_locals(body, out),
                _ => {}
            }
        }
    }
}

fn constrain_expr(expr: &Expr, env: &mut HashMap<String, InferredType>, ty: InferredType) {
    if let Expr::Ident(name) = expr {
        let current = env.get(name).cloned().unwrap_or(InferredType::Dynamic);
        env.insert(name.clone(), unify(&current, &ty));
    }
}

fn type_name(ty: &InferredType) -> String {
    match ty {
        InferredType::Int => "Int".to_string(),
        InferredType::Number => "Number".to_string(),
        InferredType::String => "String".to_string(),
        InferredType::Boolean => "Boolean".to_string(),
        InferredType::List(inner) => format!("List<{}>", type_name(inner)),
        InferredType::Map(inner) => format!("Map<{}>", type_name(inner)),
        InferredType::Struct(name) => name.clone(),
        InferredType::Enum(name) => name.clone(),
        InferredType::Dynamic => "Dynamic".to_string(),
    }
}

fn types_compatible(expected: &InferredType, actual: &InferredType) -> bool {
    match (expected, actual) {
        (InferredType::Dynamic, _) | (_, InferredType::Dynamic) => true,
        (InferredType::Int, InferredType::Int)
        | (InferredType::Number, InferredType::Number)
        | (InferredType::Number, InferredType::Int)
        | (InferredType::Int, InferredType::Number) => true,
        (InferredType::String, InferredType::String)
        | (InferredType::Boolean, InferredType::Boolean) => true,
        (InferredType::Struct(a), InferredType::Struct(b)) => a == b,
        (InferredType::Enum(a), InferredType::Enum(b)) => a == b,
        (InferredType::List(a), InferredType::List(b)) => types_compatible(a, b),
        (InferredType::Map(a), InferredType::Map(b)) => types_compatible(a, b),
        _ => false,
    }
}

fn unify(a: &InferredType, b: &InferredType) -> InferredType {
    if a == b {
        return a.clone();
    }
    match (a, b) {
        (InferredType::Int, InferredType::Number) | (InferredType::Number, InferredType::Int) => {
            InferredType::Number
        }
        (InferredType::List(ae), InferredType::List(be)) => {
            let inner = unify(ae, be);
            match inner {
                InferredType::Dynamic => InferredType::Dynamic,
                _ => InferredType::List(Box::new(inner)),
            }
        }
        (InferredType::Map(ae), InferredType::Map(be)) => {
            let inner = unify(ae, be);
            InferredType::Map(Box::new(inner))
        }
        (InferredType::Enum(a), InferredType::Enum(b)) if a == b => InferredType::Enum(a.clone()),
        (InferredType::Dynamic, other) | (other, InferredType::Dynamic) => other.clone(),
        _ => InferredType::Dynamic,
    }
}

fn is_integral_number(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0
}

fn is_numeric_type(value: &InferredType) -> bool {
    matches!(value, InferredType::Int | InferredType::Number)
}

fn fn_key(name: &str, target: Option<&str>) -> String {
    match target {
        Some(t) => format!("{}.{}", t, name),
        None => name.to_string(),
    }
}
