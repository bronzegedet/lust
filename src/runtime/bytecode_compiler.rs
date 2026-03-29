use std::collections::{HashMap, HashSet};

use crate::access::{LoweredAssignmentTarget, LoweredReadAccess};
use crate::ast::{Decl, Expr, Pattern, Stmt};
use crate::bytecode::{Chunk, Function, Instruction, Program, Value};
use crate::lowered::LoweredCall;
use crate::dispatch::BuiltinMethodKind;
use crate::expressions::{
    lower_condition_ir, ConditionKind, lower_expr_ir, LoweredBinaryExprIr, LoweredExprIr,
};
use crate::patterns::{
    lower_pattern_checks, LoweredDestructure, LoweredPatternCheck,
    LoweredPatternCheckKind, PatternAccessStep, PatternLengthCheckOp,
};
use crate::statements::{lower_stmt_ir, LoweredForStmt, LoweredMatchStmt, LoweredStmtIr};
use crate::typecheck::{InferredType, TypeInfo};

fn is_vm_builtin(name: &str) -> bool {
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
            | "open_file"
            | "list_dir"
            | "compile_lustgex"
            | "lustgex_match"
            | "lustgex_capture_builtin"
            | "regex_capture"
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
            | "__range"
            | "__range_inclusive"
            | "dict"
            | "map_values"
            | "filter_values"
            | "map_entries"
            | "filter_entries"
            | "clr"
            | "prompt"
            | "__str_trim"
            | "__str_at"
            | "__str_slice"
            | "__str_contains"
            | "__str_split"
            | "__str_to_list"
            | "__str_lines"
            | "__str_starts_with"
            | "__str_ends_with"
            | "__str_replace"
    )
}

#[derive(Debug, Clone)]
pub struct CompileError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

pub struct BytecodeCompiler {
    functions: HashMap<String, Function>,
    arities: HashMap<String, usize>,
    struct_defs: HashMap<String, Vec<String>>,
    enum_variants: HashMap<String, (String, usize)>,
    imports: HashSet<String>,
    type_info: TypeInfo,
    next_lambda_id: usize,
}

struct FunctionCompiler<'a> {
    chunk: Chunk,
    locals: HashMap<String, usize>,
    next_local: usize,
    arities: &'a HashMap<String, usize>,
    enum_variants: &'a HashMap<String, (String, usize)>,
    in_function: bool,
    self_slot: Option<usize>,
    loop_stack: Vec<LoopContext>,
    local_types: HashMap<String, InferredType>,
    type_info: &'a TypeInfo,
    globals: &'a HashSet<String>,
    functions: &'a mut HashMap<String, Function>,
    next_lambda_id: &'a mut usize,
}

struct LoopContext {
    continue_target: usize,
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
}

impl BytecodeCompiler {
    pub fn new(type_info: TypeInfo) -> Self {
        let mut enum_variants = HashMap::new();
        for variant in ["Up", "Down", "Left", "Right", "Enter", "Esc"] {
            enum_variants.insert(variant.to_string(), ("Key".to_string(), 0));
        }
        enum_variants.insert("Char".to_string(), ("Key".to_string(), 1));
        Self {
            functions: HashMap::new(),
            arities: HashMap::new(),
            struct_defs: HashMap::new(),
            enum_variants,
            imports: HashSet::new(),
            type_info,
            next_lambda_id: 0,
        }
    }

    pub fn compile(mut self, decls: &[Decl]) -> Result<Program, Vec<CompileError>> {
        let mut errors = Vec::new();

        for decl in decls {
            if let Decl::Type(name, fields) = decl {
                self.struct_defs
                    .insert(name.clone(), fields.iter().map(|(field, _)| field.clone()).collect());
            }
            if let Decl::Enum(name, variants) = decl {
                for (variant, fields) in variants {
                    self.enum_variants
                        .insert(variant.clone(), (name.clone(), fields.len()));
                }
            }
            if let Decl::Import(name) = decl {
                self.imports.insert(name.clone());
            }
            if let Decl::Fn(name, target, params, _, _) = decl {
                if let Some(target_type) = target {
                    self.arities
                        .insert(format!("{}.{}", target_type, name), params.len() + 1);
                } else {
                    self.arities.insert(name.clone(), params.len());
                }
            }
        }

        for decl in decls {
            if let Decl::Fn(name, target, params, _, body) = decl {
                let function_name = if let Some(target_type) = target {
                    format!("{}.{}", target_type, name)
                } else {
                    name.clone()
                };
                match self.compile_function(params, body, target.as_ref(), &function_name) {
                    Ok(function) => {
                        self.functions.insert(function_name, function);
                    }
                    Err(err) => errors.extend(err),
                }
            }
        }

        let main_chunk = {
            let mut main_compiler =
                FunctionCompiler::new_top_level(&self.arities, &self.enum_variants, &self.type_info, &self.imports, &mut self.functions, &mut self.next_lambda_id);
            for decl in decls {
                if let Err(err) = main_compiler.compile_decl(decl) {
                    errors.push(err);
                }
            }
            main_compiler.emit_null_return();
            main_compiler.chunk
        };
        self.functions.insert(
            "__main".to_string(),
            Function {
                arity: 0,
                chunk: main_chunk,
            },
        );

        if errors.is_empty() {
            Ok(Program {
                entry: "__main".to_string(),
                functions: self.functions,
                struct_defs: self.struct_defs,
                imports: self.imports,
            })
        } else {
            Err(errors)
        }
    }

    fn compile_function(
        &mut self,
        params: &[(String, Option<String>)],
        body: &[Stmt],
        target_type: Option<&String>,
        function_name: &str,
    ) -> Result<Function, Vec<CompileError>> {
        let mut compiler = FunctionCompiler::new_function(
            params,
            &self.arities,
            &self.enum_variants,
            target_type.is_some(),
            &self.type_info,
            &self.imports,
            function_name,
            &mut self.functions,
            &mut self.next_lambda_id,
        );
        let mut errors = Vec::new();

        for stmt in body {
            if let Err(err) = compiler.compile_stmt(stmt) {
                errors.push(err);
            }
        }

        compiler.emit_null_return();

        if errors.is_empty() {
            Ok(Function {
                arity: params.len() + usize::from(target_type.is_some()),
                chunk: compiler.chunk,
            })
        } else {
            Err(errors)
        }
    }
}

impl<'a> FunctionCompiler<'a> {
    fn new_top_level(
        arities: &'a HashMap<String, usize>,
        enum_variants: &'a HashMap<String, (String, usize)>,
        type_info: &'a TypeInfo,
        globals: &'a HashSet<String>,
        functions: &'a mut HashMap<String, Function>,
        next_lambda_id: &'a mut usize,
    ) -> Self {
        Self {
            chunk: Chunk::default(),
            locals: HashMap::new(),
            next_local: 0,
            arities,
            enum_variants,
            in_function: false,
            self_slot: None,
            loop_stack: Vec::new(),
            local_types: type_info.top_level.clone(),
            type_info,
            globals,
            functions,
            next_lambda_id,
        }
    }

    fn new_function(
        params: &[(String, Option<String>)],
        arities: &'a HashMap<String, usize>,
        enum_variants: &'a HashMap<String, (String, usize)>,
        has_self: bool,
        type_info: &'a TypeInfo,
        globals: &'a HashSet<String>,
        function_name: &str,
        functions: &'a mut HashMap<String, Function>,
        next_lambda_id: &'a mut usize,
    ) -> Self {
        let mut locals = HashMap::new();
        let mut next_local = 0;
        let mut local_types = type_info
            .functions
            .get(function_name)
            .cloned()
            .unwrap_or_default();
        let self_slot = if has_self {
            locals.insert("self".to_string(), 0);
            next_local = 1;
            if let Some((owner, _)) = function_name.split_once('.') {
                local_types
                    .entry("self".to_string())
                    .or_insert_with(|| InferredType::Struct(owner.to_string()));
            }
            Some(0)
        } else {
            None
        };
        for (idx, (name, _)) in params.iter().enumerate() {
            locals.insert(name.clone(), idx + next_local);
        }
        Self {
            chunk: Chunk::default(),
            locals,
            next_local: params.len() + next_local,
            arities,
            enum_variants,
            in_function: true,
            self_slot,
            loop_stack: Vec::new(),
            local_types,
            type_info,
            globals,
            functions,
            next_lambda_id,
        }
    }

    fn compile_lambda(&mut self, params: &[String], body: &Expr, line: usize) -> Result<(), CompileError> {
        let lambda_id = *self.next_lambda_id;
        *self.next_lambda_id += 1;
        let function_name = format!("__lambda_{}", lambda_id);

        let params_vec: Vec<(String, Option<String>)> = params.iter().map(|p| (p.clone(), None)).collect();
        
        let mut compiler = FunctionCompiler::new_function(
            &params_vec,
            self.arities,
            self.enum_variants,
            false,
            self.type_info,
            self.globals,
            &function_name,
            self.functions,
            self.next_lambda_id,
        );

        compiler.compile_expr(body, line)?;
        compiler.emit_return();

        let chunk = compiler.chunk;

        self.functions.insert(
            function_name.clone(),
            Function {
                arity: params.len(),
                chunk,
            },
        );

        self.chunk.emit(Instruction::LoadFunction(function_name));
        Ok(())
    }

    fn compile_decl(&mut self, decl: &Decl) -> Result<(), CompileError> {
        match decl {
            Decl::Stmt(stmt) => self.compile_stmt(stmt),
            Decl::Fn(_, _, _, _, _) => Ok(()),
            Decl::Import(name) => {
                if matches!(name.as_str(), "audio" | "draw" | "net") {
                    Ok(())
                } else {
                    Err(CompileError {
                        line: 1,
                        message: format!("import '{}' is not supported in the VM backend yet", name),
                    })
                }
            }
            Decl::Type(_, _) => Ok(()),
            Decl::Enum(_, _) => Ok(()),
        }
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), CompileError> {
        if let Some(lowered_stmt_ir) =
            lower_stmt_ir(
                stmt,
                &self.local_types,
                self.type_info,
                self.globals,
                !self.loop_stack.is_empty(),
            )
        {
            return self.compile_lowered_stmt_ir(&lowered_stmt_ir);
        }
        match stmt {
            Stmt::Print(line, exprs) => {
                for expr in exprs {
                    self.compile_expr(expr, *line)?;
                }
                self.chunk.emit(Instruction::Print(exprs.len()));
                Ok(())
            }
            Stmt::ExprStmt(line, expr) => {
                self.compile_expr(expr, *line)?;
                self.chunk.emit(Instruction::Pop);
                Ok(())
            }
            Stmt::Return(line, expr) => {
                self.compile_expr(expr, *line)?;
                self.chunk.emit(Instruction::Return);
                Ok(())
            }
            Stmt::Pass(_) => Ok(()),
            Stmt::Spawn(_, _, _) => Ok(()),
            Stmt::Let(_, _, _, _)
            | Stmt::Assign(_, _, _)
            | Stmt::LetPattern(_, _, _)
            | Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::If(_, _, _, _)
            | Stmt::Match(_, _, _)
            | Stmt::For(_, _, _, _, _)
            | Stmt::While(_, _, _) => {
                unreachable!("lowered stmt ir should handle structured statements before bytecode compilation")
            }
        }
    }

    fn compile_block(&mut self, stmts: &[Stmt]) -> Result<(), CompileError> {
        for stmt in stmts {
            self.compile_stmt(stmt)?;
        }
        Ok(())
    }

    fn reserve_local(&mut self) -> usize {
        let slot = self.next_local;
        self.next_local += 1;
        slot
    }

    fn emit_bool_constant(&mut self, value: bool) {
        let idx = self.chunk.add_constant(Value::Bool(value));
        self.chunk.emit(Instruction::Constant(idx));
    }

    fn emit_null_constant(&mut self) {
        let idx = self.chunk.add_constant(Value::Null);
        self.chunk.emit(Instruction::Constant(idx));
    }

    fn emit_return(&mut self) {
        self.chunk.emit(Instruction::Return);
    }

    fn emit_null_return(&mut self) {
        self.emit_null_constant();
        self.emit_return();
    }

    fn with_pattern_bindings<F>(
        &mut self,
        lowered: &LoweredDestructure,
        target_slot: usize,
        line: usize,
        f: F,
    ) -> Result<(), CompileError>
    where
        F: FnOnce(&mut Self) -> Result<(), CompileError>,
    {
        let mut saved = Vec::new();
        self.compile_pattern_bindings(lowered, target_slot, line, &mut saved)?;
        let result = f(self);
        for (name, previous) in saved.into_iter().rev() {
            if let Some(slot) = previous {
                self.locals.insert(name, slot);
            } else {
                self.locals.remove(&name);
            }
        }
        result
    }

    fn compile_lowered_stmt_ir(&mut self, stmt: &LoweredStmtIr) -> Result<(), CompileError> {
        match stmt {
            LoweredStmtIr::Let { line, lowered } => {
                self.compile_expr(&lowered.expr, *line)?;
                self.local_types
                    .insert(lowered.name.clone(), lowered.ty.clone());
                if !lowered.is_global && self.in_function {
                    let slot = self.next_local;
                    self.next_local += 1;
                    self.locals.insert(lowered.name.clone(), slot);
                    self.chunk.emit(Instruction::StoreLocal(slot));
                } else {
                    self.chunk.emit(Instruction::StoreGlobal(lowered.name.clone()));
                }
                Ok(())
            }
            LoweredStmtIr::Assign { line, lowered } => match &lowered.target {
                LoweredAssignmentTarget::Ident { name, .. } => {
                    self.compile_expr(&lowered.expr, *line)?;
                    if let Some(slot) = self.locals.get(name) {
                        self.chunk.emit(Instruction::StoreLocal(*slot));
                    } else {
                        self.chunk.emit(Instruction::StoreGlobal(name.clone()));
                    }
                    Ok(())
                }
                LoweredAssignmentTarget::TypedListIndex { target, index, .. }
                | LoweredAssignmentTarget::DynamicIndex { target, index } => {
                    self.compile_expr(target, *line)?;
                    self.compile_expr(index, *line)?;
                    self.compile_expr(&lowered.expr, *line)?;
                    self.chunk.emit(Instruction::IndexSet);
                    Ok(())
                }
                LoweredAssignmentTarget::TypedStructField { target, field, .. }
                | LoweredAssignmentTarget::DynamicMember { target, field } => {
                    self.compile_expr(target, *line)?;
                    self.compile_expr(&lowered.expr, *line)?;
                    self.chunk.emit(Instruction::MemberSet(field.clone()));
                    Ok(())
                }
                LoweredAssignmentTarget::UnsupportedSlice => Err(CompileError {
                    line: *line,
                    message: "slice expressions are not supported in the VM backend yet".to_string(),
                }),
                LoweredAssignmentTarget::Unsupported => Err(CompileError {
                    line: *line,
                    message: "only identifier, list index, and struct field assignment are supported in the VM backend yet".to_string(),
                }),
            },
            LoweredStmtIr::If {
                line,
                condition,
                then_body,
                else_body,
                has_else,
            } => {
                self.compile_condition_with_kind(&condition.expr, condition.kind, *line)?;
                let jump_to_else = self.chunk.emit(Instruction::JumpIfFalse(usize::MAX));
                self.chunk.emit(Instruction::Pop);
                self.compile_block(then_body)?;
                let jump_to_end = self.chunk.emit(Instruction::Jump(usize::MAX));
                let else_start = self.chunk.instructions.len();
                self.chunk.patch_jump(jump_to_else, else_start);
                self.chunk.emit(Instruction::Pop);
                if *has_else {
                    let else_body = else_body.as_ref().expect("lowered if reported else body");
                    self.compile_block(else_body)?;
                }
                let end = self.chunk.instructions.len();
                self.chunk.patch_jump(jump_to_end, end);
                Ok(())
            }
            LoweredStmtIr::While {
                line,
                condition,
                body,
            } => {
                let loop_start = self.chunk.instructions.len();
                self.compile_condition_with_kind(&condition.expr, condition.kind, *line)?;
                let exit_jump = self.chunk.emit(Instruction::JumpIfFalse(usize::MAX));
                self.chunk.emit(Instruction::Pop);
                self.loop_stack.push(LoopContext {
                    continue_target: loop_start,
                    break_jumps: vec![exit_jump],
                    continue_jumps: Vec::new(),
                });
                self.compile_block(body)?;
                self.chunk.emit(Instruction::Jump(loop_start));
                let exit_cleanup = self.chunk.instructions.len();
                self.chunk.emit(Instruction::Pop);
                let loop_end = self.chunk.instructions.len();
                let loop_ctx = self.loop_stack.pop().expect("loop context should exist");
                for (idx, jump) in loop_ctx.break_jumps.into_iter().enumerate() {
                    let target = if idx == 0 { exit_cleanup } else { loop_end };
                    self.chunk.patch_jump(jump, target);
                }
                Ok(())
            }
            LoweredStmtIr::For { line, lowered } => self.compile_for_stmt_ir(*line, lowered),
            LoweredStmtIr::Match { line, expr, lowered } => {
                self.compile_match_stmt_ir(*line, expr, lowered)
            }
            LoweredStmtIr::Break { line, lowered } => {
                let Some(loop_ctx) = self.loop_stack.last_mut() else {
                    return Err(CompileError {
                        line: *line,
                        message: "break is only valid inside a while loop".to_string(),
                    });
                };
                debug_assert!(lowered.valid_in_loop);
                let jump = self.chunk.emit(Instruction::Jump(usize::MAX));
                loop_ctx.break_jumps.push(jump);
                Ok(())
            }
            LoweredStmtIr::Continue { line, lowered } => {
                let Some(loop_ctx) = self.loop_stack.last_mut() else {
                    return Err(CompileError {
                        line: *line,
                        message: "continue is only valid inside a while loop".to_string(),
                    });
                };
                debug_assert!(lowered.valid_in_loop);
                if loop_ctx.continue_target == usize::MAX {
                    let jump = self.chunk.emit(Instruction::Jump(usize::MAX));
                    loop_ctx.continue_jumps.push(jump);
                } else {
                    self.chunk.emit(Instruction::Jump(loop_ctx.continue_target));
                }
                Ok(())
            }
            LoweredStmtIr::LetPattern { line, expr, lowered } => {
                self.compile_destructure_let(*line, lowered, expr)
            }
        }
    }

    fn compile_condition(&mut self, expr: &Expr, line: usize) -> Result<(), CompileError> {
        let lowered = lower_condition_ir(expr, &self.local_types, self.type_info, self.globals);
        self.compile_condition_with_kind(expr, lowered.kind, line)
    }

    fn compile_condition_with_kind(
        &mut self,
        expr: &Expr,
        kind: ConditionKind,
        line: usize,
    ) -> Result<(), CompileError> {
        match kind {
            ConditionKind::Boolean | ConditionKind::TruthyValue => self.compile_expr(expr, line),
        }
    }

    fn with_typed_bindings<F>(
        &mut self,
        binding_types: &HashMap<String, InferredType>,
        f: F,
    ) -> Result<(), CompileError>
    where
        F: FnOnce(&mut Self) -> Result<(), CompileError>,
    {
        let mut saved = Vec::new();
        for (name, ty) in binding_types {
            saved.push((name.clone(), self.local_types.insert(name.clone(), ty.clone())));
        }
        let result = f(self);
        for (name, previous) in saved.into_iter().rev() {
            if let Some(ty) = previous {
                self.local_types.insert(name, ty);
            } else {
                self.local_types.remove(&name);
            }
        }
        result
    }

    fn compile_match_stmt_ir(
        &mut self,
        line: usize,
        expr: &Expr,
        lowered: &LoweredMatchStmt,
    ) -> Result<(), CompileError> {
        let target_slot = self.reserve_local();
        self.compile_expr(expr, line)?;
        self.chunk.emit(Instruction::StoreLocal(target_slot));

        let mut end_jumps = Vec::new();
        for lowered_case in &lowered.cases {
            let case = &lowered_case.case;
            self.compile_pattern_condition(&case.pattern, target_slot, &lowered.target_type, line)?;
            let next_case_jump = self.chunk.emit(Instruction::JumpIfFalse(usize::MAX));
            self.chunk.emit(Instruction::Pop);

            self.with_pattern_bindings(&lowered_case.destructure, target_slot, line, |this| {
                this.with_typed_bindings(&lowered_case.binding_types, |this| {
                    if let Some(guard) = &case.guard {
                        this.compile_condition(guard, line)?;
                        let guard_fail_jump = this.chunk.emit(Instruction::JumpIfFalse(usize::MAX));
                        this.chunk.emit(Instruction::Pop);
                        this.compile_block(&case.body)?;
                        end_jumps.push(this.chunk.emit(Instruction::Jump(usize::MAX)));
                        let guard_fail_target = this.chunk.instructions.len();
                        this.chunk.patch_jump(guard_fail_jump, guard_fail_target);
                        this.chunk.emit(Instruction::Pop);
                    } else {
                        this.compile_block(&case.body)?;
                        end_jumps.push(this.chunk.emit(Instruction::Jump(usize::MAX)));
                    }
                    Ok(())
                })
            })?;

            let next_case_target = self.chunk.instructions.len();
            self.chunk.patch_jump(next_case_jump, next_case_target);
            self.chunk.emit(Instruction::Pop);
        }

        let end = self.chunk.instructions.len();
        for jump in end_jumps {
            self.chunk.patch_jump(jump, end);
        }
        Ok(())
    }

    fn compile_for_stmt_ir(
        &mut self,
        line: usize,
        lowered: &LoweredForStmt,
    ) -> Result<(), CompileError> {
        let iterable_slot = self.reserve_local();
        self.compile_expr(&lowered.iterable, line)?;
        self.chunk.emit(Instruction::StoreLocal(iterable_slot));

        let index_slot = self.reserve_local();
        let zero_idx = self.chunk.add_constant(Value::Number(0.0));
        self.chunk.emit(Instruction::Constant(zero_idx));
        self.chunk.emit(Instruction::StoreLocal(index_slot));

        let item_slot = self.reserve_local();
        let previous_local = self.locals.insert(lowered.item_name.clone(), item_slot);
        let previous_type = self
            .local_types
            .insert(lowered.item_name.clone(), lowered.item_ty.clone());
        let previous_index_local = lowered
            .index_name
            .as_ref()
            .and_then(|name| self.locals.insert(name.clone(), index_slot));
        let previous_index_type = lowered
            .index_name
            .as_ref()
            .and_then(|name| self.local_types.insert(name.clone(), InferredType::Number));

        let loop_start = self.chunk.instructions.len();
        self.chunk.emit(Instruction::LoadLocal(index_slot));
        self.chunk.emit(Instruction::LoadLocal(iterable_slot));
        self.chunk.emit(Instruction::ListLen);
        self.chunk.emit(Instruction::Lt);
        let exit_jump = self.chunk.emit(Instruction::JumpIfFalse(usize::MAX));
        self.chunk.emit(Instruction::Pop);

        self.chunk.emit(Instruction::LoadLocal(iterable_slot));
        self.chunk.emit(Instruction::LoadLocal(index_slot));
        self.chunk.emit(Instruction::IndexGet);
        self.chunk.emit(Instruction::StoreLocal(item_slot));

        self.loop_stack.push(LoopContext {
            continue_target: usize::MAX,
            break_jumps: vec![exit_jump],
            continue_jumps: Vec::new(),
        });
        self.compile_block(&lowered.body)?;
        let increment_start = self.chunk.instructions.len();
        if let Some(loop_ctx) = self.loop_stack.last_mut() {
            loop_ctx.continue_target = increment_start;
            for jump in &loop_ctx.continue_jumps {
                self.chunk.patch_jump(*jump, increment_start);
            }
        }
        self.chunk.emit(Instruction::LoadLocal(index_slot));
        let one_idx = self.chunk.add_constant(Value::Number(1.0));
        self.chunk.emit(Instruction::Constant(one_idx));
        self.chunk.emit(Instruction::Add);
        self.chunk.emit(Instruction::StoreLocal(index_slot));
        self.chunk.emit(Instruction::Jump(loop_start));

        let exit_cleanup = self.chunk.instructions.len();
        self.chunk.emit(Instruction::Pop);
        let loop_end = self.chunk.instructions.len();
        let loop_ctx = self.loop_stack.pop().expect("loop context should exist");
        for (idx, jump) in loop_ctx.break_jumps.into_iter().enumerate() {
            let target = if idx == 0 { exit_cleanup } else { loop_end };
            self.chunk.patch_jump(jump, target);
        }

        if let Some(previous) = previous_local {
            self.locals.insert(lowered.item_name.clone(), previous);
        } else {
            self.locals.remove(&lowered.item_name);
        }
        if let Some(previous) = previous_type {
            self.local_types.insert(lowered.item_name.clone(), previous);
        } else {
            self.local_types.remove(&lowered.item_name);
        }
        if let Some(index_name) = &lowered.index_name {
            if let Some(previous) = previous_index_local {
                self.locals.insert(index_name.clone(), previous);
            } else {
                self.locals.remove(index_name);
            }
            if let Some(previous) = previous_index_type {
                self.local_types.insert(index_name.clone(), previous);
            } else {
                self.local_types.remove(index_name);
            }
        }
        Ok(())
    }

    fn compile_pattern_condition(
        &mut self,
        pattern: &Pattern,
        target_slot: usize,
        target_ty: &InferredType,
        line: usize,
    ) -> Result<(), CompileError> {
        let checks = lower_pattern_checks(pattern, target_ty, self.type_info);
        self.compile_lowered_pattern_checks(&checks, target_slot, line)
    }

    fn emit_pattern_access(&mut self, target_slot: usize, path: &[PatternAccessStep]) {
        self.chunk.emit(Instruction::LoadLocal(target_slot));
        for step in path {
            match step {
                PatternAccessStep::StructField(field) => {
                    self.chunk.emit(Instruction::StructGetField(field.clone()));
                }
                PatternAccessStep::EnumField(_, idx) => {
                    self.chunk.emit(Instruction::EnumGetField(*idx));
                }
                PatternAccessStep::ListIndex(idx) => {
                    let const_idx = self.chunk.add_constant(Value::Number(*idx as f64));
                    self.chunk.emit(Instruction::Constant(const_idx));
                    self.chunk.emit(Instruction::IndexGet);
                }
            }
        }
    }

    fn compile_pattern_check(
        &mut self,
        check: &LoweredPatternCheck,
        target_slot: usize,
        line: usize,
    ) -> Result<(), CompileError> {
        match &check.kind {
            LoweredPatternCheckKind::AlwaysFalse => {
                self.emit_bool_constant(false);
                Ok(())
            }
            LoweredPatternCheckKind::Number(n) => {
                self.emit_pattern_access(target_slot, &check.path);
                let idx = self.chunk.add_constant(Value::Number(*n));
                self.chunk.emit(Instruction::Constant(idx));
                self.chunk.emit(Instruction::Eq);
                Ok(())
            }
            LoweredPatternCheckKind::StringLit(s) => {
                self.emit_pattern_access(target_slot, &check.path);
                let idx = self.chunk.add_constant(Value::String(s.clone()));
                self.chunk.emit(Instruction::Constant(idx));
                self.chunk.emit(Instruction::Eq);
                Ok(())
            }
            LoweredPatternCheckKind::Bool(b) => {
                self.emit_pattern_access(target_slot, &check.path);
                let idx = self.chunk.add_constant(Value::Bool(*b));
                self.chunk.emit(Instruction::Constant(idx));
                self.chunk.emit(Instruction::Eq);
                Ok(())
            }
            LoweredPatternCheckKind::Null => {
                self.emit_pattern_access(target_slot, &check.path);
                let idx = self.chunk.add_constant(Value::Null);
                self.chunk.emit(Instruction::Constant(idx));
                self.chunk.emit(Instruction::Eq);
                Ok(())
            }
            LoweredPatternCheckKind::ListLength { len, op } => {
                self.emit_pattern_access(target_slot, &check.path);
                self.chunk.emit(Instruction::IsList);
                self.emit_pattern_access(target_slot, &check.path);
                self.chunk.emit(Instruction::ListLen);
                let len_idx = self.chunk.add_constant(Value::Number(*len as f64));
                self.chunk.emit(Instruction::Constant(len_idx));
                if *op == PatternLengthCheckOp::AtLeast {
                    self.chunk.emit(Instruction::Ge);
                } else {
                    self.chunk.emit(Instruction::Eq);
                }
                self.chunk.emit(Instruction::And);
                Ok(())
            }
            LoweredPatternCheckKind::StructType(name) => {
                self.emit_pattern_access(target_slot, &check.path);
                self.chunk.emit(Instruction::StructIsType(name.clone()));
                Ok(())
            }
            LoweredPatternCheckKind::EnumVariant {
                enum_name,
                variant,
                arity,
            } => {
                let Some(enum_name) = enum_name else {
                    return Err(CompileError {
                        line,
                        message: format!("unknown enum variant '{}'", variant),
                    });
                };
                if let Some((_, expected_arity)) = self.enum_variants.get(variant) {
                    if *expected_arity != *arity {
                        return Err(CompileError {
                            line,
                            message: format!(
                                "enum variant '{}' expects {} pattern parts but got {}",
                                variant, expected_arity, arity
                            ),
                        });
                    }
                }
                self.emit_pattern_access(target_slot, &check.path);
                self.chunk
                    .emit(Instruction::EnumIsVariant(enum_name.clone(), variant.clone()));
                Ok(())
            }
        }
    }

    fn compile_pattern_bindings(
        &mut self,
        lowered: &LoweredDestructure,
        target_slot: usize,
        _line: usize,
        saved: &mut Vec<(String, Option<usize>)>,
    ) -> Result<(), CompileError> {
        for binding in &lowered.bindings {
            let slot = self.reserve_local();
            let previous = self.locals.insert(binding.name.clone(), slot);
            saved.push((binding.name.clone(), previous));
            self.emit_pattern_access(target_slot, &binding.path);
            self.chunk.emit(Instruction::StoreLocal(slot));
        }
        Ok(())
    }

    fn compile_destructure_let(
        &mut self,
        line: usize,
        lowered: &LoweredDestructure,
        expr: &Expr,
    ) -> Result<(), CompileError> {
        let target_slot = self.reserve_local();
        self.compile_expr(expr, line)?;
        self.chunk.emit(Instruction::StoreLocal(target_slot));
        self.compile_lowered_pattern_checks(&lowered.checks, target_slot, line)?;
        let mismatch_jump = self.chunk.emit(Instruction::JumpIfFalse(usize::MAX));
        self.chunk.emit(Instruction::Pop);

        for binding in &lowered.bindings {
            self.emit_pattern_access(target_slot, &binding.path);
            self.local_types
                .insert(binding.name.clone(), binding.ty.clone());
            if self.in_function {
                let slot = self.next_local;
                self.next_local += 1;
                self.locals.insert(binding.name.clone(), slot);
                self.chunk.emit(Instruction::StoreLocal(slot));
            } else {
                self.chunk.emit(Instruction::StoreGlobal(binding.name.clone()));
            }
        }

        let end_jump = self.chunk.emit(Instruction::Jump(usize::MAX));
        let mismatch_target = self.chunk.instructions.len();
        self.chunk.patch_jump(mismatch_jump, mismatch_target);
        self.chunk.emit(Instruction::Pop);
        let message_idx = self
            .chunk
            .add_constant(Value::String("destructuring pattern mismatch".to_string()));
        self.chunk.emit(Instruction::Constant(message_idx));
        self.chunk.emit(Instruction::CallBuiltin("panic".to_string(), 1));
        self.chunk.emit(Instruction::Pop);
        let end_target = self.chunk.instructions.len();
        self.chunk.patch_jump(end_jump, end_target);
        Ok(())
    }

    fn compile_lowered_pattern_checks(
        &mut self,
        checks: &[LoweredPatternCheck],
        target_slot: usize,
        line: usize,
    ) -> Result<(), CompileError> {
        if checks.is_empty() {
            self.emit_bool_constant(true);
            return Ok(());
        }
        let mut emitted_any = false;
        for check in checks {
            self.compile_pattern_check(check, target_slot, line)?;
            if emitted_any {
                self.chunk.emit(Instruction::And);
            } else {
                emitted_any = true;
            }
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr, line: usize) -> Result<(), CompileError> {
        match expr {
            Expr::Number(value) => {
                let idx = self.chunk.add_constant(Value::Number(*value));
                self.chunk.emit(Instruction::Constant(idx));
                Ok(())
            }
            Expr::StringLit(value) => {
                let idx = self.chunk.add_constant(Value::String(value.clone()));
                self.chunk.emit(Instruction::Constant(idx));
                Ok(())
            }
            Expr::Ident(name) => {
                match name.as_str() {
                    "true" => {
                        let idx = self.chunk.add_constant(Value::Bool(true));
                        self.chunk.emit(Instruction::Constant(idx));
                    }
                    "false" => {
                        let idx = self.chunk.add_constant(Value::Bool(false));
                        self.chunk.emit(Instruction::Constant(idx));
                    }
                    "null" => {
                        let idx = self.chunk.add_constant(Value::Null);
                        self.chunk.emit(Instruction::Constant(idx));
                    }
                    _ => {
                        if let Some(slot) = self.locals.get(name) {
                            self.chunk.emit(Instruction::LoadLocal(*slot));
                        } else {
                            self.chunk.emit(Instruction::LoadGlobal(name.clone()));
                        }
                    }
                }
                Ok(())
            }
            Expr::Binary(_, _, _) => {
                if let Some(LoweredExprIr::Binary(lowered)) =
                    lower_expr_ir(expr, &self.local_types, self.type_info, self.globals)
                {
                    self.compile_lowered_binary_expr(&lowered, line)
                } else {
                    unreachable!("binary expressions should lower before bytecode compilation");
                }
            }
            Expr::Call(_, _)
            | Expr::MethodCall(_, _, _)
            | Expr::Pipe(_, _ , _)
            | Expr::Index(_, _)
            | Expr::Slice(_, _, _)
            | Expr::Member(_, _) => {
                if let Some(lowered) =
                    lower_expr_ir(expr, &self.local_types, self.type_info, self.globals)
                {
                    return match lowered {
                        LoweredExprIr::Binary(_) => {
                            unreachable!("binary expressions are handled by the binary branch")
                        }
                        LoweredExprIr::Call(lowered) => self.compile_lowered_call(&lowered, line),
                        LoweredExprIr::Read(lowered) => self.compile_lowered_read_access(&lowered, line),
                    };
                }
                unreachable!("call/access expressions should lower before bytecode compilation");
            }
            Expr::List(items) => {
                for item in items {
                    self.compile_expr(item, line)?;
                }
                self.chunk.emit(Instruction::BuildList(items.len()));
                Ok(())
            }
            Expr::MapLit(items) => {
                for (key, value) in items {
                    self.compile_expr(key, line)?;
                    self.compile_expr(value, line)?;
                }
                self.chunk.emit(Instruction::CallBuiltin("dict".to_string(), items.len() * 2));
                Ok(())
            }
            Expr::StructInst(name, fields) => {
                let field_names: Vec<String> = fields.iter().map(|(field, _)| field.clone()).collect();
                for (_, value) in fields {
                    self.compile_expr(value, line)?;
                }
                self.chunk.emit(Instruction::BuildStruct(name.clone(), field_names));
                Ok(())
            }
            Expr::EnumVariant(name, args) => {
                let Some((enum_name, arity)) = self.enum_variants.get(name) else {
                    return Err(CompileError {
                        line,
                        message: format!("unknown enum variant '{}'", name),
                    });
                };
                if *arity != args.len() {
                    return Err(CompileError {
                        line,
                        message: format!(
                            "enum variant '{}' expects {} arguments but got {}",
                            name,
                            arity,
                            args.len()
                        ),
                    });
                }
                for arg in args {
                    self.compile_expr(arg, line)?;
                }
                self.chunk
                    .emit(Instruction::BuildEnum(enum_name.clone(), name.clone(), args.len()));
                Ok(())
            }
            Expr::Self_ => {
                if let Some(slot) = self.self_slot {
                    self.chunk.emit(Instruction::LoadLocal(slot));
                    Ok(())
                } else {
                    Err(CompileError {
                        line,
                        message: "self is only valid inside methods".to_string(),
                    })
                }
            }
            Expr::Lambda(params, body) => {
                self.compile_lambda(params, body, line)
            }
        }
    }

    fn compile_lowered_call(&mut self, lowered: &LoweredCall, line: usize) -> Result<(), CompileError> {
        match lowered {
            LoweredCall::BuiltinFunction { name, args } => {
                if !is_vm_builtin(name) {
                    return Err(CompileError {
                        line,
                        message: format!("call '{}' is not supported in the VM backend yet", name),
                    });
                }
                for arg in args {
                    self.compile_expr(arg, line)?;
                }
                self.chunk.emit(Instruction::CallBuiltin(name.clone(), args.len()));
                Ok(())
            }
            LoweredCall::UserFunction { name, args } => {
                let Some(expected_arity) = self.arities.get(name) else {
                    return Err(CompileError {
                        line,
                        message: format!("call '{}' is not supported in the VM backend yet", name),
                    });
                };
                if *expected_arity != args.len() {
                    return Err(CompileError {
                        line,
                        message: format!(
                            "function '{}' expects {} args but got {}",
                            name,
                            expected_arity,
                            args.len()
                        ),
                    });
                }
                for arg in args {
                    self.compile_expr(arg, line)?;
                }
                self.chunk.emit(Instruction::Call(name.clone(), args.len()));
                Ok(())
            }
            LoweredCall::BuiltinMethod { receiver, kind, args } => {
                if args.len() != kind.expected_args() {
                    return Err(CompileError {
                        line,
                        message: if kind.expected_args() == 0 {
                            "builtin method does not take arguments".to_string()
                        } else if kind.expected_args() == 1 {
                            "builtin method expects exactly one argument".to_string()
                        } else {
                            "builtin method expects exactly two arguments".to_string()
                        },
                    });
                }
                self.compile_expr(receiver, line)?;
                for arg in args {
                    self.compile_expr(arg, line)?;
                }
                match kind {
                    BuiltinMethodKind::Length => self.chunk.emit(Instruction::ListLen),
                    _ => self.chunk.emit(Instruction::CallBuiltin(
                        kind.vm_builtin_name().expect("builtin method should have vm lowering").to_string(),
                        kind.expected_args() + 1,
                    )),
                };
                Ok(())
            }
            LoweredCall::StructMethod {
                receiver,
                method_name,
                args,
                ..
            }
            | LoweredCall::DynamicMethod {
                receiver,
                method_name,
                args,
            } => {
                if method_name == "push" {
                    if args.len() != 1 {
                        return Err(CompileError {
                            line,
                            message: "push() expects exactly one argument".to_string(),
                        });
                    }
                    self.compile_expr(receiver, line)?;
                    self.compile_expr(&args[0], line)?;
                    self.chunk.emit(Instruction::ListPush);
                    return Ok(());
                }
                self.compile_expr(receiver, line)?;
                for arg in args {
                    self.compile_expr(arg, line)?;
                }
                self.chunk.emit(Instruction::CallMethod(method_name.clone(), args.len()));
                Ok(())
            }
            LoweredCall::DynamicCall { target, args } => {
                for arg in args {
                    self.compile_expr(arg, line)?;
                }
                self.compile_expr(target, line)?;
                self.chunk.emit(Instruction::CallDynamic(args.len()));
                Ok(())
            }
        }
    }

    fn compile_lowered_binary_expr(
        &mut self,
        lowered: &LoweredBinaryExprIr,
        line: usize,
    ) -> Result<(), CompileError> {
        if lowered.op == "not" {
            self.compile_expr(&lowered.right, line)?;
            self.chunk.emit(Instruction::Not);
            return Ok(());
        }

        if lowered.op == "and" {
            self.compile_expr(&lowered.left, line)?;
            let left_false_jump = self.chunk.emit(Instruction::JumpIfFalse(usize::MAX));
            self.chunk.emit(Instruction::Pop);
            self.compile_expr(&lowered.right, line)?;
            let right_false_jump = self.chunk.emit(Instruction::JumpIfFalse(usize::MAX));
            self.chunk.emit(Instruction::Pop);
            self.emit_bool_constant(true);
            let end_jump = self.chunk.emit(Instruction::Jump(usize::MAX));

            let false_target = self.chunk.instructions.len();
            self.chunk.patch_jump(left_false_jump, false_target);
            self.chunk.patch_jump(right_false_jump, false_target);
            self.chunk.emit(Instruction::Pop);
            self.emit_bool_constant(false);

            let end_target = self.chunk.instructions.len();
            self.chunk.patch_jump(end_jump, end_target);
            return Ok(());
        }

        if lowered.op == "or" {
            self.compile_expr(&lowered.left, line)?;
            let eval_right_jump = self.chunk.emit(Instruction::JumpIfFalse(usize::MAX));
            self.chunk.emit(Instruction::Pop);
            self.emit_bool_constant(true);
            let end_jump = self.chunk.emit(Instruction::Jump(usize::MAX));

            let eval_right_target = self.chunk.instructions.len();
            self.chunk.patch_jump(eval_right_jump, eval_right_target);
            self.chunk.emit(Instruction::Pop);
            self.compile_expr(&lowered.right, line)?;
            let right_false_jump = self.chunk.emit(Instruction::JumpIfFalse(usize::MAX));
            self.chunk.emit(Instruction::Pop);
            self.emit_bool_constant(true);
            let end_after_true_jump = self.chunk.emit(Instruction::Jump(usize::MAX));

            let false_target = self.chunk.instructions.len();
            self.chunk.patch_jump(right_false_jump, false_target);
            self.chunk.emit(Instruction::Pop);
            self.emit_bool_constant(false);

            let end_target = self.chunk.instructions.len();
            self.chunk.patch_jump(end_jump, end_target);
            self.chunk.patch_jump(end_after_true_jump, end_target);
            return Ok(());
        }

        self.compile_expr(&lowered.left, line)?;
        self.compile_expr(&lowered.right, line)?;
        let inst = match lowered.op.as_str() {
            "+" => Instruction::Add,
            "-" => Instruction::Sub,
            "*" => Instruction::Mul,
            "/" => Instruction::Div,
            "%" => Instruction::Mod,
            "==" => Instruction::Eq,
            "!=" => Instruction::Ne,
            ">" => Instruction::Gt,
            ">=" => Instruction::Ge,
            "<" => Instruction::Lt,
            "<=" => Instruction::Le,
            _ => {
                return Err(CompileError {
                    line,
                    message: format!(
                        "operator '{}' is not supported in the VM backend yet",
                        lowered.op
                    ),
                })
            }
        };
        self.chunk.emit(inst);
        Ok(())
    }

    fn compile_lowered_read_access(
        &mut self,
        lowered: &LoweredReadAccess,
        line: usize,
    ) -> Result<(), CompileError> {
        match lowered {
            LoweredReadAccess::TypedListIndex { target, index, .. }
            | LoweredReadAccess::StringIndex { target, index }
            | LoweredReadAccess::DynamicIndex { target, index } => {
                self.compile_expr(target, line)?;
                self.compile_expr(index, line)?;
                self.chunk.emit(Instruction::IndexGet);
                Ok(())
            }
            LoweredReadAccess::TypedListSlice { target, start, end, .. }
            | LoweredReadAccess::StringSlice { target, start, end }
            | LoweredReadAccess::DynamicSlice { target, start, end } => {
                self.compile_expr(target, line)?;
                if let Some(start) = start {
                    self.compile_expr(start, line)?;
                } else {
                    self.emit_null_constant();
                }
                if let Some(end) = end {
                    self.compile_expr(end, line)?;
                } else {
                    self.emit_null_constant();
                }
                self.chunk
                    .emit(Instruction::CallBuiltin("__slice_range".to_string(), 3));
                Ok(())
            }
            LoweredReadAccess::EnumVariantValue { enum_name, variant } => {
                self.chunk
                    .emit(Instruction::BuildEnum(enum_name.clone(), variant.clone(), 0));
                Ok(())
            }
            LoweredReadAccess::BuiltinLength { target }
            | LoweredReadAccess::DynamicMember { target, field: _ }
            | LoweredReadAccess::TypedStructField {
                target,
                struct_name: _,
                field: _,
                field_type: _,
            } => {
                let field = match lowered {
                    LoweredReadAccess::BuiltinLength { .. } => "length".to_string(),
                    LoweredReadAccess::DynamicMember { field, .. }
                    | LoweredReadAccess::TypedStructField { field, .. } => field.clone(),
                    _ => unreachable!(),
                };
                self.compile_expr(target, line)?;
                self.chunk.emit(Instruction::MemberGet(field));
                Ok(())
            }
        }
    }
}
