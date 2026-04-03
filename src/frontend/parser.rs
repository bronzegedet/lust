use crate::lexer::Lexer;
use crate::token::{SpannedToken, Token};
use crate::ast::{Decl, Expr, MatchCase, Pattern, Stmt};
use std::collections::HashSet;
use std::time::Instant;

pub struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
    pub errors: Vec<String>,
    enum_variants: HashSet<String>,
}

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        let mut enum_variants = HashSet::new();
        for variant in [
            "None",
            "Up",
            "Down",
            "Left",
            "Right",
            "Enter",
            "Esc",
            "Char",
            "FileOk",
            "FileErr",
        ] {
            enum_variants.insert(variant.to_string());
        }
        Self { tokens, pos: 0, errors: Vec::new(), enum_variants }
    }

    pub fn parse(&mut self) -> Vec<Decl> {
        let mut decls = Vec::new();
        let profile_enabled = std::env::var("LUST_PARSE_PROFILE").ok().as_deref() == Some("1");
        let trace_enabled = std::env::var("LUST_PARSE_TRACE").ok().as_deref() == Some("1");
        let started = Instant::now();
        let mut last_report_pos = 0usize;
        while self.peek().is_some() {
            let start_pos = self.pos;
            let start_line = self.current_line();
            let start_token = self.peek();
            let decl_started = if profile_enabled { Some(Instant::now()) } else { None };
            if trace_enabled {
                eprintln!(
                    "[parse-trace] top-level start pos={} line={} token={:?}",
                    start_pos,
                    start_line,
                    start_token
                );
            }
            if let Some(decl) = self.parse_declaration() {
                decls.push(decl);
            }
            if self.pos == start_pos {
                self.advance();
            }
            if trace_enabled {
                eprintln!(
                    "[parse-trace] top-level end pos={} next={:?} errors={}",
                    self.pos,
                    self.peek(),
                    self.errors.len()
                );
            }
            if let Some(decl_started) = decl_started {
                let elapsed = decl_started.elapsed().as_secs_f64();
                if elapsed >= 0.25 {
                    eprintln!(
                        "[parse +{:>6.2}s] decl line={} start={:?} end_pos={} took {:>6.2}s",
                        started.elapsed().as_secs_f64(),
                        start_line,
                        start_token,
                        self.pos,
                        elapsed
                    );
                }
            }
            if profile_enabled && self.pos >= last_report_pos + 250 {
                last_report_pos = self.pos;
                eprintln!(
                    "[parse +{:>6.2}s] pos={}/{} decls={} line={} token={:?}",
                    started.elapsed().as_secs_f64(),
                    self.pos,
                    self.tokens.len(),
                    decls.len(),
                    self.current_line(),
                    self.peek()
                );
            }
        }
        if profile_enabled {
            eprintln!(
                "[parse +{:>6.2}s] done decls={} errors={}",
                started.elapsed().as_secs_f64(),
                decls.len(),
                self.errors.len()
            );
        }
        decls
    }

    fn report_error(&mut self, msg: &str) {
        let token_str = if let Some(t) = self.peek() { format!("{:?}", t) } else { "EOF".to_string() };
        self.errors.push(format!("Lust Error on line {}: {} (found {})", self.current_line(), msg, token_str));
    }

    fn parse_interpolated_string(&mut self, raw: String) -> Expr {
        if !raw.contains("${") {
            return Expr::StringLit(raw);
        }

        let mut parts = Vec::new();
        let mut cursor = 0usize;

        while let Some(rel_start) = raw[cursor..].find("${") {
            let start = cursor + rel_start;
            if start > cursor {
                parts.push(Expr::StringLit(raw[cursor..start].to_string()));
            }

            let mut depth = 1usize;
            let mut idx = start + 2;
            let mut in_string = false;
            let mut escaped = false;
            let mut end = None;

            while idx < raw.len() {
                let ch = raw.as_bytes()[idx] as char;
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if ch == '\\' {
                        escaped = true;
                    } else if ch == '"' {
                        in_string = false;
                    }
                    idx += 1;
                    continue;
                }

                match ch {
                    '"' => in_string = true,
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(idx);
                            break;
                        }
                    }
                    _ => {}
                }
                idx += 1;
            }

            let Some(end_idx) = end else {
                self.errors.push(format!(
                    "Lust Error on line {}: Unterminated string interpolation",
                    self.current_line()
                ));
                return Expr::StringLit(raw);
            };

            let expr_src = raw[start + 2..end_idx].trim();
            let expr = self.parse_interpolation_expr(expr_src);
            parts.push(Expr::Call("to_string".to_string(), vec![expr]));
            cursor = end_idx + 1;
        }

        if cursor < raw.len() {
            parts.push(Expr::StringLit(raw[cursor..].to_string()));
        }

        let mut iter = parts.into_iter();
        let Some(mut expr) = iter.next() else {
            return Expr::StringLit(String::new());
        };
        for part in iter {
            expr = Expr::Binary(Box::new(expr), "+".to_string(), Box::new(part));
        }
        expr
    }

    fn parse_interpolation_expr(&mut self, src: &str) -> Expr {
        let mut lexer = Lexer::new(src);
        let mut tokens = Vec::new();
        while let Some(token) = lexer.next_token() {
            tokens.push(token);
        }

        let mut nested = Parser::new(tokens);
        nested.enum_variants = self.enum_variants.clone();
        let expr = nested.parse_expr();
        if nested.peek().is_some() {
            nested.report_error("Unexpected tokens inside string interpolation");
        }
        self.errors.extend(
            nested
                .errors
                .into_iter()
                .map(|err| format!("{} (inside string interpolation)", err)),
        );
        expr
    }

    fn parse_spawn(&mut self) -> Option<Stmt> {
        let line = self.current_line();
        self.advance(); // spawn
        if let Some(Token::Ident(name)) = self.advance() {
            if matches!(self.advance(), Some(Token::LParen)) {
                let mut args = Vec::new();
                while let Some(t) = self.peek() {
                    if matches!(t, Token::RParen) { break; }
                    args.push(self.parse_expr());
                    if matches!(self.peek(), Some(Token::Comma)) { self.advance(); }
                }
                self.advance(); // )
                return Some(Stmt::Spawn(line, name, args));
            }
        }
        self.report_error("Expected function call after spawn");
        None
    }

    fn parse_declaration(&mut self) -> Option<Decl> {
        match self.peek() {
            Some(Token::Fn) => self.parse_fn(),
            Some(Token::Enum) => self.parse_enum(),
            Some(Token::Type | Token::Struct) => self.parse_type(),
            Some(Token::Import) => self.parse_import(),
            _ => self.parse_statement().map(Decl::Stmt),
        }
    }

    fn parse_fn(&mut self) -> Option<Decl> {
        self.advance(); // fn
        let mut name = String::new();
        let mut target_type = None;
        if let Some(id) = self.parse_identifier() {
            if matches!(self.peek(), Some(Token::Dot)) {
                self.advance(); // .
                if let Some(method_name) = self.parse_identifier() {
                    target_type = Some(id);
                    name = method_name;
                }
            } else {
                name = id;
            }
            
            if let Some(Token::LParen) = self.advance() {
                let mut args = Vec::new();
                while let Some(t) = self.peek() {
                    if matches!(t, Token::RParen) { break; }
                    if let Some(arg) = self.parse_identifier() {
                        let arg_ty = self.parse_optional_type_annotation();
                        args.push((arg, arg_ty));
                    } else {
                        self.report_error("Expected parameter name");
                        return None;
                    }
                    if let Some(Token::Comma) = self.peek() { self.advance(); }
                }
                if matches!(self.advance(), Some(Token::RParen)) {
                    let ret_type = self.parse_optional_return_type();
                    let body = self.parse_block();
                    return Some(Decl::Fn(name, target_type, args, ret_type, body));
                } else {
                    self.report_error("Expected ')' after parameters");
                }
            } else {
                self.report_error("Expected '(' after function name");
            }
        } else {
            self.report_error("Expected function name");
        }
        None
    }

    fn parse_type(&mut self) -> Option<Decl> {
        self.advance(); // type or struct
        if let Some(Token::Ident(name)) = self.advance() {
            if matches!(self.peek(), Some(Token::Equals)) {
                self.advance();
            }
            if !matches!(self.peek(), Some(Token::LBrace)) {
                self.report_error("Expected '{' after type name");
                return None;
            }
            self.advance(); // {
            let mut fields = Vec::new();
            while let Some(t) = self.peek() {
                if matches!(t, Token::RBrace) { break; }
                let start_pos = self.pos;
                if let Some(field) = self.parse_identifier() {
                    let field_ty = self.parse_optional_type_annotation();
                    fields.push((field, field_ty));
                }
                if self.pos == start_pos {
                    self.report_error("Expected type field");
                    break;
                }
                if matches!(self.peek(), Some(Token::Comma)) { self.advance(); }
            }
            if !matches!(self.peek(), Some(Token::RBrace)) {
                self.report_error("Expected '}' after type fields");
                return None;
            }
            self.advance(); // }
            return Some(Decl::Type(name, fields));
        }
        None
    }

    fn parse_enum(&mut self) -> Option<Decl> {
        self.advance(); // enum
        let name = match self.advance() {
            Some(Token::Ident(name)) => name,
            _ => {
                self.report_error("Expected enum name");
                return None;
            }
        };
        if matches!(self.peek(), Some(Token::Equals)) {
            self.advance();
        }
        let mut variants = Vec::new();
        loop {
            let variant = match self.advance() {
                Some(Token::Ident(name)) => name,
                _ => {
                    self.report_error("Expected enum variant name");
                    return None;
                }
            };
            let mut fields = Vec::new();
            if matches!(self.peek(), Some(Token::LParen)) {
                self.advance();
                while let Some(t) = self.peek() {
                    if matches!(t, Token::RParen) { break; }
                    let field = self.parse_identifier().or_else(|| {
                        self.report_error("Expected enum variant field name");
                        None
                    })?;
                    fields.push(field);
                    if matches!(self.peek(), Some(Token::Comma)) { self.advance(); }
                }
                if !matches!(self.advance(), Some(Token::RParen)) {
                    self.report_error("Expected ')' after enum variant fields");
                    return None;
                }
            }
            variants.push((variant, fields));
            if matches!(self.peek(), Some(Token::Pipe)) {
                self.advance();
            } else {
                break;
            }
        }
        for (variant, _) in &variants {
            self.enum_variants.insert(variant.clone());
        }
        Some(Decl::Enum(name, variants))
    }

    fn parse_import(&mut self) -> Option<Decl> {
        self.advance(); // import
        if let Some(Token::StringLit(file) | Token::RawStringLit(file)) = self.advance() {
            return Some(Decl::Import(file));
        }
        None
    }

    fn parse_block(&mut self) -> Vec<Stmt> {
        let mut statements = Vec::new();
        while let Some(t) = self.peek() {
            if matches!(t, Token::End | Token::Else) { break; }
            let start_pos = self.pos;
            if let Some(stmt) = self.parse_statement() {
                statements.push(stmt);
            }
            if self.pos == start_pos {
                self.advance();
            }
        }
        self.advance(); // consume end/else/etc (Wait, else should be handled by caller)
        statements
    }

    fn parse_statement(&mut self) -> Option<Stmt> {
        let line = self.current_line();
        match self.peek() {
            Some(Token::Let) => {
                self.advance();
                if matches!(self.peek(), Some(Token::LBracket)) {
                    let pattern = self.parse_pattern()?;
                    if matches!(self.advance(), Some(Token::Equals)) {
                        let expr = self.parse_expr();
                        return Some(Stmt::LetPattern(line, pattern, expr));
                    }
                    self.report_error("Expected '=' after destructuring pattern");
                    return None;
                }
                if let Some(Token::Ident(name)) = self.advance() {
                    let declared_ty = self.parse_optional_type_annotation();
                    if matches!(self.advance(), Some(Token::Equals)) {
                        let expr = self.parse_expr();
                        return Some(Stmt::Let(line, name, declared_ty, expr));
                    }
                }
                None
            }
            Some(Token::Pass) => {
                self.advance();
                Some(Stmt::Pass(line))
            }
            Some(Token::Return) => {
                self.advance();
                let expr = match self.peek() {
                    Some(Token::End | Token::Else) | None => Expr::Ident("null".to_string()),
                    _ => self.parse_expr(),
                };
                Some(Stmt::Return(line, expr))
            }
            Some(Token::Break) => {
                self.advance();
                Some(Stmt::Break(line))
            }
            Some(Token::Continue) => {
                self.advance();
                Some(Stmt::Continue(line))
            }
            Some(Token::Print) => {
                self.advance();
                if matches!(self.advance(), Some(Token::LParen)) {
                    let mut exprs = Vec::new();
                    while let Some(t) = self.peek() {
                        if matches!(t, Token::RParen) { break; }
                        let start_pos = self.pos;
                        exprs.push(self.parse_expr());
                        if self.pos == start_pos {
                            self.report_error("Expected print argument");
                            break;
                        }
                        if matches!(self.peek(), Some(Token::Plus | Token::Comma)) { self.advance(); }
                    }
                    self.advance(); // )
                    Some(Stmt::Print(line, exprs))
                } else {
                    None
                }
            }
            Some(Token::If) => {
                self.advance();
                let cond = self.parse_expr();
                if matches!(self.advance(), Some(Token::Then)) {
                    let mut if_body = Vec::new();
                    while let Some(t) = self.peek() {
                        if matches!(t, Token::End | Token::Else) { break; }
                        let start_pos = self.pos;
                        if let Some(stmt) = self.parse_statement() { if_body.push(stmt); }
                        if self.pos == start_pos { self.advance(); }
                    }
                    let mut else_body = None;
                    let mut has_same_line_else_if = false;
                    if let Some(Token::Else) = self.peek() {
                        let else_line = self.current_line();
                        self.advance();
                        if matches!(self.peek(), Some(Token::If)) && self.current_line() == else_line {
                            // Treat `else if` as sugar only when both keywords are on the same line.
                            has_same_line_else_if = true;
                            if let Some(if_stmt) = self.parse_statement() {
                                else_body = Some(vec![if_stmt]);
                            }
                        } else {
                            let mut body = Vec::new();
                            while let Some(t) = self.peek() {
                                if matches!(t, Token::End) { break; }
                                let start_pos = self.pos;
                                if let Some(stmt) = self.parse_statement() { body.push(stmt); }
                                if self.pos == start_pos { self.advance(); }
                            }
                            else_body = Some(body);
                        }
                    }
                    // Same-line `else if` is parsed recursively and consumes the shared `end`
                    // for the entire chain. Plain `else` blocks still own their own trailing end
                    // even if the body happens to be a single nested `if`.
                    if !has_same_line_else_if && matches!(self.peek(), Some(Token::End)) {
                        self.advance();
                    }
                    Some(Stmt::If(line, cond, if_body, else_body))
                } else {
                    None
                }
            }
            Some(Token::Match) => {
                self.advance();
                let target = self.parse_expr();
                if !matches!(self.advance(), Some(Token::Do)) {
                    self.report_error("Expected 'do' after match expression");
                    return None;
                }
                let mut cases = Vec::new();
                while matches!(self.peek(), Some(Token::Case)) {
                    self.advance();
                    let pattern = self.parse_pattern()?;
                    let guard = if matches!(self.peek(), Some(Token::If)) {
                        self.advance();
                        Some(self.parse_expr())
                    } else {
                        None
                    };
                    if !matches!(self.advance(), Some(Token::Then)) {
                        self.report_error("Expected 'then' after case pattern");
                        return None;
                    }
                    let mut body = Vec::new();
                    while let Some(t) = self.peek() {
                        if matches!(t, Token::Case | Token::End) { break; }
                        let start_pos = self.pos;
                        if let Some(stmt) = self.parse_statement() { body.push(stmt); }
                        if self.pos == start_pos { self.advance(); }
                    }
                    cases.push(MatchCase { pattern, guard, body });
                }
                if matches!(self.peek(), Some(Token::End)) {
                    self.advance();
                } else {
                    self.report_error("Expected 'end' after match");
                    return None;
                }
                Some(Stmt::Match(line, target, cases))
            }
            Some(Token::Spawn) => self.parse_spawn(),
            Some(Token::While) => {
                self.advance();
                let cond = self.parse_expr();
                if matches!(self.advance(), Some(Token::Do)) {
                    let mut body = Vec::new();
                    while let Some(t) = self.peek() {
                        if matches!(t, Token::End) { break; }
                        let start_pos = self.pos;
                        if let Some(stmt) = self.parse_statement() { body.push(stmt); }
                        if self.pos == start_pos { self.advance(); }
                    }
                    self.advance(); // end
                    Some(Stmt::While(line, cond, body))
                } else {
                    self.report_error("Expected 'do' after while condition");
                    None
                }
            }
            Some(Token::For) => {
                self.advance();
                if matches!(self.peek(), Some(Token::LBracket)) {
                    let pattern = self.parse_pattern()?;
                    if !matches!(self.advance(), Some(Token::In)) {
                        self.report_error("Expected 'in' after for loop pattern");
                        return None;
                    }
                    let iterable_start = self.parse_expr();
                    let iterable = match self.peek() {
                        Some(Token::DotDot) => {
                            self.advance();
                            let iterable_end = self.parse_expr();
                            Expr::Call("__range".to_string(), vec![iterable_start, iterable_end])
                        }
                        Some(Token::DotDotEq) => {
                            self.advance();
                            let iterable_end = self.parse_expr();
                            Expr::Call("__range_inclusive".to_string(), vec![iterable_start, iterable_end])
                        }
                        _ => iterable_start,
                    };
                    if !matches!(self.advance(), Some(Token::Do)) {
                        self.report_error("Expected 'do' after for loop iterable");
                        return None;
                    }
                    let mut body = Vec::new();
                    while let Some(t) = self.peek() {
                        if matches!(t, Token::End) { break; }
                        let start_pos = self.pos;
                        if let Some(stmt) = self.parse_statement() { body.push(stmt); }
                        if self.pos == start_pos { self.advance(); }
                    }
                    self.advance(); // end
                    let temp_name = format!("__for_item_{}", line);
                    let mut lowered_body = Vec::with_capacity(body.len() + 1);
                    lowered_body.push(Stmt::LetPattern(line, pattern, Expr::Ident(temp_name.clone())));
                    lowered_body.extend(body);
                    return Some(Stmt::For(line, None, temp_name, iterable, lowered_body));
                }
                let Some(first_name) = self.parse_identifier() else {
                    self.report_error("Expected loop variable after 'for'");
                    return None;
                };
                let (index_name, item_name) = if matches!(self.peek(), Some(Token::Comma)) {
                    self.advance();
                    let Some(second_name) = self.parse_identifier() else {
                        self.report_error("Expected item variable after ',' in for loop");
                        return None;
                    };
                    (Some(first_name), second_name)
                } else {
                    (None, first_name)
                };
                if !matches!(self.advance(), Some(Token::In)) {
                    self.report_error("Expected 'in' after for loop variable");
                    return None;
                }
                let iterable_start = self.parse_expr();
                let iterable = match self.peek() {
                    Some(Token::DotDot) => {
                        self.advance();
                        let iterable_end = self.parse_expr();
                        Expr::Call("__range".to_string(), vec![iterable_start, iterable_end])
                    }
                    Some(Token::DotDotEq) => {
                        self.advance();
                        let iterable_end = self.parse_expr();
                        Expr::Call("__range_inclusive".to_string(), vec![iterable_start, iterable_end])
                    }
                    _ => iterable_start,
                };
                if !matches!(self.advance(), Some(Token::Do)) {
                    self.report_error("Expected 'do' after for loop iterable");
                    return None;
                }
                let mut body = Vec::new();
                while let Some(t) = self.peek() {
                    if matches!(t, Token::End) { break; }
                    let start_pos = self.pos;
                    if let Some(stmt) = self.parse_statement() { body.push(stmt); }
                    if self.pos == start_pos { self.advance(); }
                }
                self.advance(); // end
                Some(Stmt::For(line, index_name, item_name, iterable, body))
            }
            _ => {
                let start_pos = self.pos;
                let expr = self.parse_expr();
                if self.pos == start_pos {
                    self.report_error("Expected statement");
                    return None;
                }
                match self.peek() {
                    Some(Token::Equals) => {
                        self.advance();
                        let val = self.parse_expr();
                        Some(Stmt::Assign(line, expr, val))
                    }
                    Some(Token::PlusEq) => {
                        self.advance();
                        let val = self.parse_expr();
                        Some(self.parse_compound_assign_stmt(line, expr, "+", val))
                    }
                    Some(Token::MinusEq) => {
                        self.advance();
                        let val = self.parse_expr();
                        Some(self.parse_compound_assign_stmt(line, expr, "-", val))
                    }
                    Some(Token::StarEq) => {
                        self.advance();
                        let val = self.parse_expr();
                        Some(self.parse_compound_assign_stmt(line, expr, "*", val))
                    }
                    Some(Token::SlashEq) => {
                        self.advance();
                        let val = self.parse_expr();
                        Some(self.parse_compound_assign_stmt(line, expr, "/", val))
                    }
                    Some(Token::PercentEq) => {
                        self.advance();
                        let val = self.parse_expr();
                        Some(self.parse_compound_assign_stmt(line, expr, "%", val))
                    }
                    _ => Some(Stmt::ExprStmt(line, expr)),
                }
            }
        }
    }

    fn parse_compound_assign_stmt(&self, line: usize, target: Expr, op: &str, value: Expr) -> Stmt {
        let desugared = Expr::Binary(Box::new(target.clone()), op.to_string(), Box::new(value));
        Stmt::Assign(line, target, desugared)
    }

    fn parse_expr(&mut self) -> Expr {
        self.parse_pipe()
    }

    fn parse_pipe(&mut self) -> Expr {
        let mut expr = self.parse_logic_or();
        while matches!(self.peek(), Some(Token::PipeForward)) {
            self.advance();
            let Some((name, args)) = self.parse_pipe_step() else {
                self.report_error("Expected pipe step like name(...) after '|>'");
                break;
            };
            expr = Expr::Pipe(Box::new(expr), name, args);
        }
        expr
    }

    fn parse_pipe_step(&mut self) -> Option<(String, Vec<Expr>)> {
        let name = self.parse_identifier()?;
        if !matches!(self.advance(), Some(Token::LParen)) {
            return None;
        }
        let mut args = Vec::new();
        while let Some(t) = self.peek() {
            if matches!(t, Token::RParen) { break; }
            let start_pos = self.pos;
            args.push(self.parse_expr());
            if self.pos == start_pos {
                self.report_error("Expected pipe argument");
                break;
            }
            if matches!(self.peek(), Some(Token::Comma)) { self.advance(); }
        }
        if !matches!(self.advance(), Some(Token::RParen)) {
            self.report_error("Expected ')' after pipe step arguments");
            return None;
        }
        Some((name, args))
    }

    fn parse_logic_or(&mut self) -> Expr {
        let mut expr = self.parse_logic_and();
        while let Some(Token::Or) = self.peek() {
            self.advance();
            let right = self.parse_logic_and();
            expr = Expr::Binary(Box::new(expr), "or".to_string(), Box::new(right));
        }
        expr
    }

    fn parse_logic_and(&mut self) -> Expr {
        let mut expr = self.parse_equality();
        while let Some(Token::And) = self.peek() {
            self.advance();
            let right = self.parse_equality();
            expr = Expr::Binary(Box::new(expr), "and".to_string(), Box::new(right));
        }
        expr
    }

    fn parse_equality(&mut self) -> Expr {
        let mut expr = self.parse_comparison();
        while let Some(t) = self.peek() {
            if matches!(t, Token::Eq | Token::Ne) {
                let op = if matches!(t, Token::Eq) { "==" } else { "!=" }.to_string();
                self.advance();
                let right = self.parse_comparison();
                expr = Expr::Binary(Box::new(expr), op, Box::new(right));
            } else {
                break;
            }
        }
        expr
    }

    fn parse_comparison(&mut self) -> Expr {
        let mut expr = self.parse_addition();
        while let Some(t) = self.peek() {
            if matches!(t, Token::Gt | Token::Lt | Token::Ge | Token::Le) {
                let op = match t {
                    Token::Gt => ">",
                    Token::Lt => "<",
                    Token::Ge => ">=",
                    Token::Le => "<=",
                    _ => unreachable!(),
                }.to_string();
                self.advance();
                let right = self.parse_addition();
                expr = Expr::Binary(Box::new(expr), op, Box::new(right));
            } else {
                break;
            }
        }
        expr
    }

    fn parse_addition(&mut self) -> Expr {
        let mut expr = self.parse_multiplication();
        while let Some(t) = self.peek() {
            if matches!(t, Token::Plus | Token::Minus) {
                let op = if matches!(t, Token::Plus) { "+" } else { "-" }.to_string();
                self.advance();
                let right = self.parse_multiplication();
                expr = Expr::Binary(Box::new(expr), op, Box::new(right));
            } else {
                break;
            }
        }
        expr
    }

    fn parse_multiplication(&mut self) -> Expr {
        let mut expr = self.parse_unary_not();
        while let Some(t) = self.peek() {
            if matches!(t, Token::Star | Token::Slash | Token::Percent) {
                let op = match t {
                    Token::Star => "*",
                    Token::Slash => "/",
                    Token::Percent => "%",
                    _ => unreachable!(),
                }.to_string();
                self.advance();
                let right = self.parse_primary();
                expr = Expr::Binary(Box::new(expr), op, Box::new(right));
            } else {
                break;
            }
        }
        expr
    }

    fn parse_unary_not(&mut self) -> Expr {
        if let Some(Token::Not) = self.peek() {
            self.advance();
            let right = self.parse_unary_not();
            return Expr::Binary(Box::new(Expr::Number(0.0)), "not".to_string(), Box::new(right));
        }
        // Unary minus: -expr becomes (0 - expr)
        if let Some(Token::Minus) = self.peek() {
            self.advance();
            let right = self.parse_primary();
            return Expr::Binary(Box::new(Expr::Number(0.0)), "-".to_string(), Box::new(right));
        }
        self.parse_primary()
    }

    fn parse_struct_fields_with_update(
        &mut self,
        type_name: &str,
        trace_enabled: bool,
    ) -> (Vec<(String, Expr)>, Option<Box<Expr>>) {
        let mut fields = Vec::new();
        let mut base = None;
        while let Some(t) = self.peek() {
            if matches!(t, Token::RBrace) {
                break;
            }
            let start_pos = self.pos;
            if matches!(self.peek(), Some(Token::DotDot)) {
                self.advance(); // ..
                let spread_expr = self.parse_expr();
                base = Some(Box::new(spread_expr));
                if matches!(self.peek(), Some(Token::Comma)) {
                    self.advance();
                }
                continue;
            }
            if let Some(f) = self.parse_identifier() {
                if trace_enabled {
                    eprintln!(
                        "[parse-trace] struct {} field {:?} at pos={} next={:?}",
                        type_name,
                        f,
                        self.pos,
                        self.peek()
                    );
                }
                if matches!(self.advance(), Some(Token::Colon)) {
                    let val = self.parse_expr();
                    fields.push((f, val));
                    if trace_enabled {
                        eprintln!(
                            "[parse-trace] struct {} field done pos={} next={:?}",
                            type_name,
                            self.pos,
                            self.peek()
                        );
                    }
                }
            }
            if self.pos == start_pos {
                self.report_error("Expected struct field");
                break;
            }
            if matches!(self.peek(), Some(Token::Comma)) {
                self.advance();
            }
        }
        (fields, base)
    }

    fn parse_primary(&mut self) -> Expr {
        let mut expr = match self.advance() {
            Some(Token::Number(n)) => Expr::Number(n),
            Some(Token::StringLit(s)) => self.parse_interpolated_string(s),
            Some(Token::RawStringLit(s)) => Expr::StringLit(s),
            Some(Token::Fn) => self.parse_lambda_expr(),
            Some(Token::Self_) => Expr::Self_,
            Some(Token::Ident(id)) => {
                // Check if it's a struct instantiation: Name { ... }
                if let Some(Token::LBrace) = self.peek() {
                    let trace_enabled = std::env::var("LUST_PARSE_TRACE").ok().as_deref() == Some("1");
                    self.advance(); // {
                    let (fields, base) = self.parse_struct_fields_with_update(&id, trace_enabled);
                    if trace_enabled {
                        eprintln!("[parse-trace] struct {} closing pos={} next={:?}", id, self.pos, self.peek());
                    }
                    if !matches!(self.peek(), Some(Token::RBrace)) {
                        self.report_error("Expected '}' after struct fields");
                    } else {
                        self.advance(); // }
                    }
                    Expr::StructInst(id, fields, base)
                } else if self.enum_variants.contains(&id) {
                    Expr::EnumVariant(id, Vec::new())
                } else {
                    Expr::Ident(id)
                }
            }
            Some(Token::LBracket) => {
                let mut items = Vec::new();
                while let Some(t) = self.peek() {
                    if matches!(t, Token::RBracket) { break; }
                    let start_pos = self.pos;
                    items.push(self.parse_expr());
                    if self.pos == start_pos {
                        self.report_error("Expected list item");
                        break;
                    }
                    if matches!(self.peek(), Some(Token::Comma)) { self.advance(); }
                }
                self.advance(); // ]
                Expr::List(items)
            }
            Some(Token::LBrace) => {
                let mut items = Vec::new();
                while let Some(t) = self.peek() {
                    if matches!(t, Token::RBrace) { break; }
                    let start_pos = self.pos;
                    let key = self.parse_expr();
                    if !matches!(self.peek(), Some(Token::Colon)) {
                        self.report_error("Expected ':' after map key");
                        break;
                    }
                    self.advance(); // :
                    let value = self.parse_expr();
                    items.push((key, value));
                    if self.pos == start_pos {
                        self.report_error("Expected map entry");
                        break;
                    }
                    if matches!(self.peek(), Some(Token::Comma)) { self.advance(); }
                }
                self.advance(); // }
                Expr::MapLit(items)
            }
            Some(Token::LParen) => {
                let expr = self.parse_expr();
                self.advance(); // )
                expr
            }
            _ => Expr::Number(0.0), // Placeholder for error
        };

        // Handle suffixes (Dot access, Indexing, Calls)
        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    self.advance();
                    if let Some(field) = self.parse_identifier() {
                        expr = Expr::Member(Box::new(expr), field);
                    }
                }
                Some(Token::LBracket) => {
                    self.advance();
                    expr = self.parse_bracket_suffix(expr);
                }
                Some(Token::LParen) => {
                    self.advance(); // (
                    let mut args = Vec::new();
                    while let Some(t) = self.peek() {
                        if matches!(t, Token::RParen) { break; }
                        let start_pos = self.pos;
                        args.push(self.parse_expr());
                        if self.pos == start_pos {
                            self.report_error("Expected call argument");
                            break;
                        }
                        if matches!(self.peek(), Some(Token::Comma)) { self.advance(); }
                    }
                    self.advance(); // )
                    if let Expr::Member(obj, name) = expr {
                        expr = Expr::MethodCall(obj, name, args);
                    } else if let Expr::Ident(name) = expr {
                        expr = Expr::Call(name, args);
                    } else if let Expr::EnumVariant(ref name, ref existing_args) = expr {
                        if existing_args.is_empty() {
                            expr = Expr::EnumVariant(name.clone(), args);
                        }
                    } else {
                        // Fallback for anonymous function calls if added later
                    }
                }
                Some(Token::LBrace) => {
                    self.advance(); // {
                    let trace_enabled = std::env::var("LUST_PARSE_TRACE").ok().as_deref() == Some("1");
                    if let Expr::Ident(name) = expr.clone() {
                        let (fields, base) = self.parse_struct_fields_with_update(&name, trace_enabled);
                        if !matches!(self.peek(), Some(Token::RBrace)) {
                            self.report_error("Expected '}' after struct fields");
                        } else {
                            self.advance(); // }
                        }
                        expr = Expr::StructInst(name, fields, base);
                    } else {
                        let _ = self.parse_struct_fields_with_update("<expr>", trace_enabled);
                        if matches!(self.peek(), Some(Token::RBrace)) {
                            self.advance(); // }
                        }
                        self.report_error("Struct literal target must be a type name");
                    }
                }
               _ => break,
            }
        }
        expr
    }

    fn parse_bracket_suffix(&mut self, target: Expr) -> Expr {
        let start = if matches!(self.peek(), Some(Token::DotDot | Token::RBracket)) {
            None
        } else {
            Some(self.parse_expr())
        };

        if matches!(self.peek(), Some(Token::DotDot)) {
            self.advance();
            let end = if matches!(self.peek(), Some(Token::RBracket)) {
                None
            } else {
                Some(self.parse_expr())
            };
            if !matches!(self.advance(), Some(Token::RBracket)) {
                self.report_error("Expected ']' after slice expression");
            }
            Expr::Slice(
                Box::new(target),
                start.map(Box::new),
                end.map(Box::new),
            )
        } else {
            let Some(index) = start else {
                self.report_error("Expected index or slice expression inside '[]'");
                if !matches!(self.advance(), Some(Token::RBracket)) {
                    self.report_error("Expected ']' after index expression");
                }
                return Expr::Index(Box::new(target), Box::new(Expr::Number(0.0)));
            };
            if !matches!(self.advance(), Some(Token::RBracket)) {
                self.report_error("Expected ']' after index expression");
            }
            Expr::Index(Box::new(target), Box::new(index))
        }
    }

    fn parse_lambda_expr(&mut self) -> Expr {
        if !matches!(self.advance(), Some(Token::LParen)) {
            self.report_error("Expected '(' after 'fn' in lambda");
            return Expr::Number(0.0);
        }
        let mut params = Vec::new();
        while let Some(t) = self.peek() {
            if matches!(t, Token::RParen) { break; }
            let Some(param) = self.parse_identifier() else {
                self.report_error("Expected lambda parameter name");
                return Expr::Number(0.0);
            };
            params.push(param);
            if matches!(self.peek(), Some(Token::Comma)) {
                self.advance();
            }
        }
        if !matches!(self.advance(), Some(Token::RParen)) {
            self.report_error("Expected ')' after lambda parameters");
            return Expr::Number(0.0);
        }
        if !matches!(self.advance(), Some(Token::FatArrow)) {
            self.report_error("Expected '=>' after lambda parameters");
            return Expr::Number(0.0);
        }
        let body = self.parse_expr();
        Expr::Lambda(params, Box::new(body))
    }

    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.pos).map(|t| t.kind.clone())
    }

    fn parse_identifier(&mut self) -> Option<String> {
        match self.peek() {
            Some(Token::Ident(id)) => { self.advance(); Some(id) }
            Some(Token::Self_) => { self.advance(); Some("self".to_string()) }
            Some(Token::Type) => { self.advance(); Some("type".to_string()) }
            Some(Token::Struct) => { self.advance(); Some("struct".to_string()) }
            _ => None
        }
    }

    fn parse_type_name(&mut self) -> Option<String> {
        let base = self.parse_identifier()?;
        if matches!(self.peek(), Some(Token::Lt)) {
            self.advance();
            let inner = self.parse_type_name().or_else(|| {
                self.report_error("Expected type name inside generic annotation");
                None
            })?;
            if !matches!(self.advance(), Some(Token::Gt)) {
                self.report_error("Expected '>' after generic type annotation");
                return None;
            }
            return Some(format!("{base}<{inner}>"));
        }
        Some(base)
    }

    fn parse_optional_type_annotation(&mut self) -> Option<String> {
        if matches!(self.peek(), Some(Token::Colon)) {
            self.advance();
            return self.parse_type_name();
        }
        None
    }

    fn parse_optional_return_type(&mut self) -> Option<String> {
        let saved = self.pos;
        if matches!(self.peek(), Some(Token::Minus)) {
            self.advance();
            if matches!(self.peek(), Some(Token::Gt)) {
                self.advance();
                if let Some(ty) = self.parse_type_name() {
                    return Some(ty);
                }
                self.report_error("Expected return type after '->'");
                return None;
            }
        }
        self.pos = saved;
        None
    }

    fn parse_pattern(&mut self) -> Option<Pattern> {
        match self.advance() {
            Some(Token::LBracket) => {
                let mut items = Vec::new();
                let mut has_rest = false;
                while let Some(t) = self.peek() {
                    if matches!(t, Token::RBracket) { break; }
                    if matches!(t, Token::DotDot) {
                        self.advance();
                        has_rest = true;
                        if !matches!(self.peek(), Some(Token::RBracket)) {
                            self.report_error("List rest pattern '..' is only allowed at the end");
                            return None;
                        }
                        break;
                    }
                    items.push(self.parse_pattern()?);
                    if matches!(self.peek(), Some(Token::Comma)) { self.advance(); }
                }
                if !matches!(self.advance(), Some(Token::RBracket)) {
                    self.report_error("Expected ']' after list pattern");
                    return None;
                }
                Some(Pattern::List(items, has_rest))
            }
            Some(Token::Ident(id)) => {
                if id == "_" {
                    Some(Pattern::Wildcard)
                } else if id == "true" {
                    Some(Pattern::Bool(true))
                } else if id == "false" {
                    Some(Pattern::Bool(false))
                } else if id == "null" {
                    Some(Pattern::Null)
                } else if matches!(self.peek(), Some(Token::LBrace)) {
                    self.advance();
                    let mut fields = Vec::new();
                    while let Some(t) = self.peek() {
                        if matches!(t, Token::RBrace) { break; }
                        let field = self.parse_identifier()?;
                        if !matches!(self.advance(), Some(Token::Colon)) {
                            self.report_error("Expected ':' in struct pattern");
                            return None;
                        }
                        let pat = self.parse_pattern()?;
                        fields.push((field, pat));
                        if matches!(self.peek(), Some(Token::Comma)) { self.advance(); }
                    }
                    if !matches!(self.advance(), Some(Token::RBrace)) {
                        self.report_error("Expected '}' after struct pattern");
                        return None;
                    }
                    Some(Pattern::Struct(id, fields))
                } else if self.enum_variants.contains(&id) && matches!(self.peek(), Some(Token::LParen)) {
                    self.advance();
                    let mut parts = Vec::new();
                    while let Some(t) = self.peek() {
                        if matches!(t, Token::RParen) { break; }
                        parts.push(self.parse_pattern()?);
                        if matches!(self.peek(), Some(Token::Comma)) { self.advance(); }
                    }
                    if !matches!(self.advance(), Some(Token::RParen)) {
                        self.report_error("Expected ')' after enum variant pattern");
                        return None;
                    }
                    Some(Pattern::EnumVariant(id, parts))
                } else if self.enum_variants.contains(&id) {
                    Some(Pattern::EnumVariant(id, Vec::new()))
                } else {
                    Some(Pattern::Bind(id))
                }
            }
            Some(Token::StringLit(s)) => Some(Pattern::StringLit(s)),
            Some(Token::RawStringLit(s)) => Some(Pattern::StringLit(s)),
            Some(Token::Number(n)) => Some(Pattern::Number(n)),
            _ => {
                self.report_error("Expected pattern");
                None
            }
        }
    }

    fn advance(&mut self) -> Option<Token> {
        let t = self.peek();
        if t.is_some() { self.pos += 1; }
        t
    }

    fn current_line(&self) -> usize {
        self.tokens.get(self.pos).map(|t| t.line).or_else(|| self.tokens.last().map(|t| t.line)).unwrap_or(1)
    }

}

#[cfg(test)]
mod tests {
    use super::Parser;
    use crate::ast::{Decl, Expr, Stmt};
    use crate::lexer::Lexer;

    fn parse_single_stmt(source: &str) -> Stmt {
        let mut lexer = Lexer::new(source);
        let mut tokens = Vec::new();
        while let Some(token) = lexer.next_token() {
            tokens.push(token);
        }
        let mut parser = Parser::new(tokens);
        let decls = parser.parse();
        assert!(parser.errors.is_empty(), "unexpected parser errors: {:?}", parser.errors);
        match decls.as_slice() {
            [Decl::Stmt(stmt)] => stmt.clone(),
            other => panic!("expected one statement, got {:?}", other),
        }
    }

    #[test]
    fn parses_plus_equals_as_desugared_assignment() {
        let stmt = parse_single_stmt("count += 1");
        match stmt {
            Stmt::Assign(_, Expr::Ident(name), Expr::Binary(left, op, right)) => {
                assert_eq!(name, "count");
                assert_eq!(op, "+");
                assert!(matches!(*left, Expr::Ident(ref left_name) if left_name == "count"));
                assert!(matches!(*right, Expr::Number(value) if value == 1.0));
            }
            other => panic!("unexpected stmt: {:?}", other),
        }
    }

    #[test]
    fn parses_raw_strings_without_interpolation() {
        let stmt = parse_single_stmt(r##"print(r#"${name} "quoted""#)"##);
        match stmt {
            Stmt::Print(_, exprs) => {
                assert_eq!(exprs.len(), 1);
                assert!(matches!(&exprs[0], Expr::StringLit(text) if text == "${name} \"quoted\""));
            }
            other => panic!("unexpected stmt: {:?}", other),
        }
    }

    #[test]
    fn parses_raw_string_import_paths() {
        let mut lexer = Lexer::new(r##"import r#"std/lustgex"#"##);
        let mut tokens = Vec::new();
        while let Some(token) = lexer.next_token() {
            tokens.push(token);
        }
        let mut parser = Parser::new(tokens);
        let decls = parser.parse();
        assert!(parser.errors.is_empty(), "unexpected parser errors: {:?}", parser.errors);
        match decls.as_slice() {
            [Decl::Import(path)] => assert_eq!(path, "std/lustgex"),
            other => panic!("expected one import, got {:?}", other),
        }
    }

    #[test]
    fn parses_struct_update_literal_with_base_spread() {
        let stmt = parse_single_stmt("let updated = Pair { right: 9, ..base }");
        match stmt {
            Stmt::Let(_, name, _, Expr::StructInst(struct_name, fields, Some(base))) => {
                assert_eq!(name, "updated");
                assert_eq!(struct_name, "Pair");
                assert_eq!(fields.len(), 1);
                assert!(matches!(fields[0], (ref f, Expr::Number(v)) if f == "right" && v == 9.0));
                assert!(matches!(*base, Expr::Ident(ref id) if id == "base"));
            }
            other => panic!("unexpected stmt: {:?}", other),
        }
    }
}
