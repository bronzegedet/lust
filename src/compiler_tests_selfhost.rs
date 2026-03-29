#[cfg(test)]
mod tests {
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::typecheck::TypeChecker;
    use crate::bytecode_compiler::BytecodeCompiler;
    use crate::vm::Vm;
    use std::fs;
    use std::fs::File;
    use std::path::PathBuf;
    use std::process::{Command, Output, Stdio};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn parse_snippet(src: &str) -> Vec<crate::ast::Decl> {
        let mut lexer = Lexer::new(src);
        let mut tokens = Vec::new();
        while let Some(t) = lexer.next_token() {
            tokens.push(t);
        }

        let mut parser = Parser::new(tokens);
        let decls = parser.parse();
        assert!(parser.errors.is_empty(), "parse errors: {:?}", parser.errors);
        decls
    }

    fn run_vm_snippet(src: &str) -> Vm {
        run_vm_snippet_with_args(src, vec![])
    }

    fn run_vm_snippet_with_args(src: &str, args: Vec<String>) -> Vm {
        run_vm_snippet_with_args_and_keys(src, args, vec![])
    }

    fn run_vm_snippet_with_args_and_keys(src: &str, args: Vec<String>, key_inputs: Vec<String>) -> Vm {
        run_vm_snippet_with_runtime_inputs(src, args, key_inputs, vec![])
    }

    fn run_vm_snippet_with_runtime_inputs(
        src: &str,
        args: Vec<String>,
        key_inputs: Vec<String>,
        input_lines: Vec<String>,
    ) -> Vm {
        let mut lexer = Lexer::new(src);
        let mut tokens = Vec::new();
        while let Some(t) = lexer.next_token() {
            tokens.push(t);
        }

        let mut parser = Parser::new(tokens);
        let decls = parser.parse();
        assert!(parser.errors.is_empty(), "parse errors: {:?}", parser.errors);

        let type_info = TypeChecker::new()
            .check(&decls)
            .expect("typecheck failed");
        let chunk = BytecodeCompiler::new(type_info)
            .compile(&decls)
            .expect("bytecode compile failed");
        let mut vm = Vm::new_with_args_keys_and_input(chunk, args, key_inputs, input_lines);
        vm.run().expect("vm run failed");
        vm
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn fresh_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{}_{}", name, nonce));
        fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    fn lust_cli_command() -> Command {
        let mut command = Command::new("cargo");
        command
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args(["run", "--quiet", "--bin", "lust", "--"]);
        command
    }

    fn lock_lust_cli() -> std::sync::MutexGuard<'static, ()> {
        static LUST_CLI_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LUST_CLI_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn run_command_with_timeout(
        mut command: Command,
        timeout: Duration,
        stage: &str,
    ) -> Output {
        let capture_dir = fresh_temp_dir("lust_command_capture");
        let stdout_path = capture_dir.join("stdout.txt");
        let stderr_path = capture_dir.join("stderr.txt");
        let stdout_file = File::create(&stdout_path)
            .unwrap_or_else(|e| panic!("failed to create {} stdout capture: {}", stage, e));
        let stderr_file = File::create(&stderr_path)
            .unwrap_or_else(|e| panic!("failed to create {} stderr capture: {}", stage, e));
        let mut child = command
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn {}: {}", stage, e));

        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let stdout = fs::read(&stdout_path)
                        .unwrap_or_else(|e| panic!("failed to read {} stdout: {}", stage, e));
                    let stderr = fs::read(&stderr_path)
                        .unwrap_or_else(|e| panic!("failed to read {} stderr: {}", stage, e));
                    return Output { status, stdout, stderr };
                }
                Ok(None) => {
                    if started.elapsed() >= timeout {
                        let _ = child.kill();
                        let status = child
                            .wait()
                            .unwrap_or_else(|e| panic!("failed to collect timed out {} status: {}", stage, e));
                        let output = Output {
                            status,
                            stdout: fs::read(&stdout_path).unwrap_or_default(),
                            stderr: fs::read(&stderr_path).unwrap_or_default(),
                        };
                        panic!(
                            "{} timed out after {:.1}s\nstdout:\n{}\nstderr:\n{}",
                            stage,
                            timeout.as_secs_f64(),
                            String::from_utf8_lossy(&output.stdout),
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => panic!("failed to wait on {}: {}", stage, e),
            }
        }
    }

    fn parse_source(src: &str) -> Vec<crate::ast::Decl> {
        let mut lexer = Lexer::new(src);
        let mut tokens = Vec::new();
        while let Some(t) = lexer.next_token() {
            tokens.push(t);
        }

        let mut parser = Parser::new(tokens);
        let decls = parser.parse();
        assert!(parser.errors.is_empty(), "parse errors: {:?}", parser.errors);
        decls
    }

    fn expr_bridge_signature(expr: &crate::ast::Expr) -> String {
        match expr {
            crate::ast::Expr::Number(_) => "Number".to_string(),
            crate::ast::Expr::StringLit(_) => "String".to_string(),
            crate::ast::Expr::Ident(name) => format!("ident:{name}"),
            crate::ast::Expr::Binary(left, op, right) => format!(
                "binary({} {} {})",
                expr_bridge_signature(left),
                op,
                expr_bridge_signature(right)
            ),
            crate::ast::Expr::Call(name, args) => {
                let arg_sigs = args.iter().map(expr_bridge_signature).collect::<Vec<_>>().join(",");
                format!("call({name};{arg_sigs})")
            }
            crate::ast::Expr::Lambda(params, body) => {
                format!("lambda({};{})", params.join(","), expr_bridge_signature(body))
            }
            crate::ast::Expr::Pipe(target, name, args) => {
                let arg_sigs = args.iter().map(expr_bridge_signature).collect::<Vec<_>>().join(",");
                format!("pipe({}|>{};{})", expr_bridge_signature(target), name, arg_sigs)
            }
            crate::ast::Expr::MethodCall(receiver, name, args) => {
                let arg_sigs = args.iter().map(expr_bridge_signature).collect::<Vec<_>>().join(",");
                format!("method({}.{};{})", expr_bridge_signature(receiver), name, arg_sigs)
            }
            crate::ast::Expr::StructInst(name, fields) => {
                let field_sigs = fields
                    .iter()
                    .map(|(field, expr)| format!("{field}:{}", expr_bridge_signature(expr)))
                    .collect::<Vec<_>>()
                    .join(",");
                format!("struct({name};{field_sigs})")
            }
            crate::ast::Expr::EnumVariant(name, args) => {
                let arg_sigs = args.iter().map(expr_bridge_signature).collect::<Vec<_>>().join(",");
                format!("enum({name};{arg_sigs})")
            }
            crate::ast::Expr::List(items) => {
                let item_sigs = items.iter().map(expr_bridge_signature).collect::<Vec<_>>().join(",");
                format!("list({item_sigs})")
            }
            crate::ast::Expr::MapLit(items) => {
                let item_sigs = items
                    .iter()
                    .map(|(key, value)| format!("{}:{}", expr_bridge_signature(key), expr_bridge_signature(value)))
                    .collect::<Vec<_>>()
                    .join(",");
                format!("map({item_sigs})")
            }
            crate::ast::Expr::Member(target, name) => format!("member({}.{})", expr_bridge_signature(target), name),
            crate::ast::Expr::Index(target, idx) => {
                format!("index({}[{}])", expr_bridge_signature(target), expr_bridge_signature(idx))
            }
            crate::ast::Expr::Slice(target, start, end) => {
                let start_sig = start
                    .as_ref()
                    .map(|expr| expr_bridge_signature(expr))
                    .unwrap_or_default();
                let end_sig = end
                    .as_ref()
                    .map(|expr| expr_bridge_signature(expr))
                    .unwrap_or_default();
                format!("slice({}[{}..{}])", expr_bridge_signature(target), start_sig, end_sig)
            }
            crate::ast::Expr::Self_ => "self".to_string(),
        }
    }

    fn extract_tiny_typecheck_bridge(
        initial_bindings: &[(String, String)],
        body: &[crate::ast::Stmt],
    ) -> (Vec<&'static str>, Vec<usize>, Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
        let mut stmt_kinds = Vec::new();
        let mut stmt_lines = Vec::new();
        let mut stmt_names = Vec::new();
        let mut declared_types = Vec::new();
        let mut actual_types = Vec::new();
        let mut expr_shapes = Vec::new();
        let mut known_types = std::collections::HashMap::new();

        for (name, ty) in initial_bindings {
            known_types.insert(name.clone(), ty.clone());
        }

        for stmt in body {
            match stmt {
                crate::ast::Stmt::Let(line, name, declared_ty, expr) => {
                    stmt_kinds.push("let");
                    stmt_lines.push(*line);
                    stmt_names.push(name.clone());
                    declared_types.push(declared_ty.clone().unwrap_or_default());
                    let actual = match expr {
                        crate::ast::Expr::Ident(ident) => known_types.get(ident).cloned().unwrap_or_default(),
                        crate::ast::Expr::StringLit(_) => "String".to_string(),
                        crate::ast::Expr::Number(_) => "Number".to_string(),
                        _ => "Dynamic".to_string(),
                    };
                    actual_types.push(actual.clone());
                    expr_shapes.push(expr_bridge_signature(expr));
                    let binding_ty = if let Some(declared) = declared_ty.as_ref() {
                        declared.clone()
                    } else {
                        actual.to_string()
                    };
                    if !binding_ty.is_empty() {
                        known_types.insert(name.clone(), binding_ty);
                    }
                }
                crate::ast::Stmt::Return(line, expr) => {
                    stmt_kinds.push("return");
                    stmt_lines.push(*line);
                    stmt_names.push(String::new());
                    declared_types.push(String::new());
                    let actual = match expr {
                        crate::ast::Expr::Ident(ident) => known_types.get(ident).cloned().unwrap_or_default(),
                        crate::ast::Expr::StringLit(_) => "String".to_string(),
                        crate::ast::Expr::Number(_) => "Number".to_string(),
                        _ => "Dynamic".to_string(),
                    };
                    actual_types.push(actual);
                    expr_shapes.push(expr_bridge_signature(expr));
                }
                crate::ast::Stmt::If(line, cond, _, _) => {
                    stmt_kinds.push("if");
                    stmt_lines.push(*line);
                    stmt_names.push(String::new());
                    declared_types.push(String::new());
                    actual_types.push(String::new());
                    expr_shapes.push(expr_bridge_signature(cond));
                }
                _ => {}
            }
        }

        (stmt_kinds, stmt_lines, stmt_names, declared_types, actual_types, expr_shapes)
    }

    fn encode_bridge_items(items: &[String]) -> String {
        items
            .iter()
            .map(|item| if item.is_empty() { "@".to_string() } else { item.clone() })
            .collect::<Vec<_>>()
            .join("|")
    }

    fn encode_bridge_kinds(items: &[&'static str]) -> String {
        items.join("|")
    }

    fn encode_bridge_lines(items: &[usize]) -> String {
        items.iter().map(|line| line.to_string()).collect::<Vec<_>>().join("|")
    }

    #[derive(Debug, Clone)]
    struct ReducedBridgeFn {
        fn_name: String,
        param_names: Vec<String>,
        param_types: Vec<String>,
        stmt_kinds: Vec<&'static str>,
        stmt_lines: Vec<usize>,
        stmt_names: Vec<String>,
        declared_types: Vec<String>,
        actual_types: Vec<String>,
        ret_type: String,
        expr_shapes: Vec<String>,
        known_fn_names: Vec<String>,
        known_fn_param_types: Vec<String>,
        known_fn_ret_types: Vec<String>,
        struct_field_structs: Vec<String>,
        struct_field_names: Vec<String>,
        struct_field_types: Vec<String>,
    }

    fn reduced_bridge_fn_from_source(src: &str, fn_name: &str) -> ReducedBridgeFn {
        let decls = parse_source(src);
        let known_fn_names = decls
            .iter()
            .filter_map(|decl| match decl {
                crate::ast::Decl::Fn(name, None, _, _, _) => Some(name.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let known_fn_param_types = decls
            .iter()
            .filter_map(|decl| match decl {
                crate::ast::Decl::Fn(_, None, params, _, _) => Some(
                    params
                        .iter()
                        .map(|(_, ty)| ty.clone().unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join(","),
                ),
                _ => None,
            })
            .collect::<Vec<_>>();
        let known_fn_ret_types = decls
            .iter()
            .filter_map(|decl| match decl {
                crate::ast::Decl::Fn(_, None, _, ret_type, _) => Some(ret_type.clone().unwrap_or_default()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut struct_field_structs = Vec::new();
        let mut struct_field_names = Vec::new();
        let mut struct_field_types = Vec::new();
        for decl in &decls {
            if let crate::ast::Decl::Type(name, fields) = decl {
                for (field_name, field_ty) in fields {
                    struct_field_structs.push(name.clone());
                    struct_field_names.push(field_name.clone());
                    struct_field_types.push(field_ty.clone().unwrap_or_default());
                }
            }
        }
        let (params, ret_type, body) = decls
            .iter()
            .find_map(|decl| match decl {
                crate::ast::Decl::Fn(name, None, params, ret_type, body) if name == fn_name => {
                    Some((params.clone(), ret_type.clone(), body.clone()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{} function not found", fn_name));

        let param_names = params.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>();
        let param_types = params
            .iter()
            .map(|(_, ty)| ty.clone().unwrap_or_default())
            .collect::<Vec<_>>();
        let initial_bindings = param_names
            .iter()
            .cloned()
            .zip(param_types.iter().cloned())
            .collect::<Vec<_>>();
        let (stmt_kinds, stmt_lines, stmt_names, declared_types, actual_types, expr_shapes) =
            extract_tiny_typecheck_bridge(&initial_bindings, &body);

        ReducedBridgeFn {
            fn_name: fn_name.to_string(),
            param_names,
            param_types,
            stmt_kinds,
            stmt_lines,
            stmt_names,
            declared_types,
            actual_types,
            ret_type: ret_type.unwrap_or_default(),
            expr_shapes,
            known_fn_names,
            known_fn_param_types,
            known_fn_ret_types,
            struct_field_structs,
            struct_field_names,
            struct_field_types,
        }
    }

    fn run_reduced_bridge_fn(bridge: &ReducedBridgeFn) -> Output {
        run_reduced_bridge_fn_with_backend(bridge, "run")
    }

    fn run_reduced_bridge_fn_with_backend(bridge: &ReducedBridgeFn, backend: &str) -> Output {
        let root = repo_root();
        let _lust_cli_guard = lock_lust_cli();
        let mut command = lust_cli_command();
        command
            .current_dir(&root)
            .args([
                backend,
                "lust_src/typecheck.lust",
                "bridge_fn",
                &bridge.fn_name,
                &encode_bridge_items(&bridge.param_names),
                &encode_bridge_items(&bridge.param_types),
                &encode_bridge_kinds(&bridge.stmt_kinds),
                &encode_bridge_lines(&bridge.stmt_lines),
                &encode_bridge_items(&bridge.stmt_names),
                &encode_bridge_items(&bridge.declared_types),
                &encode_bridge_items(&bridge.actual_types),
                &bridge.ret_type,
                &encode_bridge_items(&bridge.expr_shapes),
                &encode_bridge_items(&bridge.known_fn_names),
                &encode_bridge_items(&bridge.known_fn_param_types),
                &encode_bridge_items(&bridge.known_fn_ret_types),
                &encode_bridge_items(&bridge.struct_field_structs),
                &encode_bridge_items(&bridge.struct_field_names),
                &encode_bridge_items(&bridge.struct_field_types),
            ])
            .output()
            .expect("failed to run lust-side reduced bridge mode")
    }

    #[test]
    fn parses_scan_string_function_without_leaking_trailing_statements() {
        let source = fs::read_to_string(repo_root().join("lust_src/parser.lust"))
            .expect("failed to read lust_src/parser.lust");
        let decls = parse_source(&source);

        let scan_string = decls
            .iter()
            .find_map(|decl| match decl {
                crate::ast::Decl::Fn(name, None, _, _, body) if name == "scan_string" => Some(body),
                _ => None,
            })
            .expect("scan_string function not found");

        assert_eq!(scan_string.len(), 7, "unexpected scan_string body: {:?}", scan_string);
        assert!(matches!(scan_string[4], crate::ast::Stmt::While(_, _, _)));
        assert!(matches!(scan_string[5], crate::ast::Stmt::If(_, _, _, _)));
        assert!(matches!(scan_string[6], crate::ast::Stmt::Return(_, _)));

        let boot_print_idx = decls
            .iter()
            .position(|decl| matches!(
                decl,
                crate::ast::Decl::Stmt(crate::ast::Stmt::Print(_, exprs))
                    if exprs.len() == 1
            ))
            .expect("boot print not found");

        for decl in &decls[..boot_print_idx] {
            assert!(
                !matches!(decl, crate::ast::Decl::Stmt(_)),
                "unexpected top-level statement before boot print: {:?}",
                decl
            );
        }
    }

    #[test]
    fn parses_run_parser_as_a_function_before_bootstrap_statements() {
        let source = fs::read_to_string(repo_root().join("lust_src/parser.lust"))
            .expect("failed to read lust_src/parser.lust");
        let decls = parse_source(&source);

        let run_parser_idx = decls
            .iter()
            .position(|decl| matches!(decl, crate::ast::Decl::Fn(name, None, _, _, _) if name == "run_parser"))
            .expect("run_parser function not found");

        let boot_print_idx = decls
            .iter()
            .position(|decl| matches!(
                decl,
                crate::ast::Decl::Stmt(crate::ast::Stmt::Print(_, exprs))
                    if exprs.len() == 1
            ))
            .expect("boot print not found");

        assert!(
            run_parser_idx < boot_print_idx,
            "run_parser should be declared before bootstrap statements: {:?}",
            decls
        );
    }

    #[test]
    fn parses_string_interpolation_into_expression_tree() {
        let decls = parse_snippet(
            r#"
let name = "Ada"
let msg = "hello ${name} ${1 + 2}"
"#,
        );

        let expr = decls
            .iter()
            .find_map(|decl| match decl {
                crate::ast::Decl::Stmt(crate::ast::Stmt::Let(_, name, _, expr)) if name == "msg" => Some(expr),
                _ => None,
            })
            .expect("msg let not found");

        assert!(
            !matches!(expr, crate::ast::Expr::StringLit(_)),
            "interpolated string should lower into a composed expression: {:?}",
            expr
        );
    }

    #[test]
    fn parses_pipe_and_lambda_expression_tree() {
        let decls = parse_source(
            r#"
let clean = raw_line |> trim() |> split("|") |> filter(fn(x) => x.length() > 0) |> map(fn(x) => x.trim())
"#,
        );

        let expr = decls
            .iter()
            .find_map(|decl| match decl {
                crate::ast::Decl::Stmt(crate::ast::Stmt::Let(_, name, _, expr)) if name == "clean" => Some(expr),
                _ => None,
            })
            .expect("clean let not found");

        let sig = expr_bridge_signature(expr);
        assert!(sig.contains("pipe("), "unexpected signature: {}", sig);
        assert!(sig.contains("lambda(x;"), "unexpected signature: {}", sig);
        assert!(sig.contains("|>trim"), "unexpected signature: {}", sig);
        assert!(sig.contains("|>filter"), "unexpected signature: {}", sig);
        assert!(sig.contains("|>map"), "unexpected signature: {}", sig);
    }

    #[test]
    fn parses_destructuring_let_pattern() {
        let decls = parse_source(
            r#"
let [code, _, qty] = "SKU|ignored|3" |> split("|")
"#,
        );

        let (pattern, expr) = decls
            .iter()
            .find_map(|decl| match decl {
                crate::ast::Decl::Stmt(crate::ast::Stmt::LetPattern(_, pattern, expr)) => Some((pattern, expr)),
                _ => None,
            })
            .expect("destructuring let not found");

        match pattern {
            crate::ast::Pattern::List(items, has_rest) => {
                assert_eq!(items.len(), 3, "unexpected item count: {:?}", items);
                assert!(!has_rest, "did not expect trailing rest");
                assert!(matches!(&items[0], crate::ast::Pattern::Bind(name) if name == "code"));
                assert!(matches!(&items[1], crate::ast::Pattern::Wildcard));
                assert!(matches!(&items[2], crate::ast::Pattern::Bind(name) if name == "qty"));
            }
            other => panic!("expected list pattern, got {:?}", other),
        }

        let sig = expr_bridge_signature(expr);
        assert!(sig.contains("pipe("), "unexpected signature: {}", sig);
        assert!(sig.contains("|>split"), "unexpected signature: {}", sig);
    }

    #[test]
    fn parses_index_slice_expression_tree() {
        let decls = parse_source(
            r#"
let prefix = "ORD20260327_USR99"[0..3]
let date = "ORD20260327_USR99"[3..11]
let user = "ORD20260327_USR99"[12..]
let all = "ORD20260327_USR99"[..]
"#,
        );

        let signatures = decls
            .iter()
            .filter_map(|decl| match decl {
                crate::ast::Decl::Stmt(crate::ast::Stmt::Let(_, name, _, expr)) => {
                    Some((name.clone(), expr_bridge_signature(expr)))
                }
                _ => None,
            })
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(signatures.get("prefix").map(String::as_str), Some("slice(String[Number..Number])"));
        assert_eq!(signatures.get("date").map(String::as_str), Some("slice(String[Number..Number])"));
        assert_eq!(signatures.get("user").map(String::as_str), Some("slice(String[Number..])"));
        assert_eq!(signatures.get("all").map(String::as_str), Some("slice(String[..])"));
    }

    #[path = "compiler_tests_vm.rs"]
    mod vm_core;

    #[test]
    fn run_uses_script_directory_for_relative_file_reads() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_run_relative_paths");
        let script_path = dir.join("main.lust");
        let data_path = dir.join("payload.txt");
        fs::write(&data_path, "hello from sibling file").expect("failed to write payload");
        fs::write(
            &script_path,
            r#"
let content = read_file("payload.txt")
print(content)
"#,
        )
        .expect("failed to write lust script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run lust CLI");

        assert!(
            output.status.success(),
            "lust run failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("hello from sibling file"),
            "unexpected stdout:\n{}",
            stdout
        );
    }

    #[test]
    fn run_executes_csv_generator_example() {
        let root = repo_root();
        let script_path = root.join("examples/generate_csv.lust");
        let dir = fresh_temp_dir("lust_run_generate_csv");
        let output_path = dir.join("users.csv");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
                output_path.to_str().unwrap(),
                "250",
            ])
            .output()
            .expect("failed to run csv generator example");

        assert!(
            output.status.success(),
            "csv generator example failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("generated 250 rows"), "unexpected stdout:\n{}", stdout);
        assert!(
            stdout.contains(&format!("output {}", output_path.display())),
            "unexpected stdout:\n{}",
            stdout
        );

        let csv = fs::read_to_string(&output_path).expect("failed to read generated csv");
        let lines = csv.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 251, "unexpected csv contents:\n{}", csv);
        assert_eq!(lines[0], "id,user_key,segment,plan,city,score,active,signup_day");
        assert_eq!(lines[1], "1,user_000001,alpha,enterprise,seattle,28,true,2026-01-01");
        assert_eq!(lines[250], "250,user_000250,omega,team,chicago,261,true,2026-10-26");
    }

    #[test]
    fn run_executes_string_interpolation_program() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_run_string_interpolation");
        let script_path = dir.join("string_interpolation.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let name = "Ada"
    let score = 7
    print("user=${name} total=${score + 1}")
end

main()
"#,
        )
        .expect("failed to write interpolation script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run interpolation script");

        assert!(
            output.status.success(),
            "interpolation script failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("user=Ada total=8"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn run_executes_string_lines_program() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_run_string_lines");
        let script_path = dir.join("string_lines.lust");
        let input_path = dir.join("report.txt");

        fs::write(&input_path, "alpha\nbeta\ngamma\n").expect("failed to write report file");
        fs::write(
            &script_path,
            format!(
                r#"
fn main()
    let content = read_file("{}")
    let lines = content.lines()
    let i = 0
    while i < lines.length() do
        print("line=${{i}}:${{lines[i]}}")
        i = i + 1
    end
end

main()
"#,
                input_path.display()
            ),
        )
        .expect("failed to write string lines script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run string lines script");

        assert!(
            output.status.success(),
            "string lines script failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("line=0:alpha"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("line=1:beta"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("line=2:gamma"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn run_executes_reconciliation_example() {
        let root = repo_root();
        let script_path = root.join("examples/reconcile_report.lust");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run reconciliation example");

        assert!(
            output.status.success(),
            "reconciliation example failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("item=A-100 desc=Widget Red system=12 physical=12"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("item=B-205 desc=Sprocket Blue system=4 physical=3"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("order=78421 company=Jolly Industrial Supply"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("items=3 system_total=35 physical_total=34 mismatches=1"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn run_executes_for_loop_and_pass_example() {
        let root = repo_root();
        let script_path = root.join("examples/std_report_parse_demo.lust");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run std report parse demo");

        assert!(
            output.status.success(),
            "std report parse demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("order=78421 company=Jolly Industrial Supply"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("item=A-100 desc=Widget Red system=12 physical=12"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("item=B-205 desc=Sprocket Blue system=4 physical=3"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("lustgex=^\\s+\\d+.*?$"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn run_executes_lustgex_match_program() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_run_lustgex_match");
        let script_path = dir.join("lustgex_match_demo.lust");

        fs::write(
            &script_path,
            r#"
import "std/lustgex"

fn main()
    let order_pattern = "start then \"BR/ORD#\" then fewest anything"
    let row_pattern = "start then fewest anything then \"|\" then fewest anything then \"|\" then integer then \"|\" then integer then end"
    print("order=${lustgex_match(\"BR/ORD#|78421|Jolly Industrial Supply\", order_pattern)}")
    print("row=${lustgex_matches(\"A-100|Widget Red|12|12\", row_pattern)}")
    print("miss=${lustgex_match(\"misc text before items\", row_pattern)}")
end

main()
"#,
        )
        .expect("failed to write lustgex match script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run lustgex match script");

        assert!(
            output.status.success(),
            "lustgex match script failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("order=true"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("row=true"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("miss=false"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn run_executes_string_parsing_helper_program() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_run_string_parsing_helpers");
        let script_path = dir.join("string_parsing_helpers.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let header = "BR/ORD#|78421|Jolly Industrial Supply"
    let footer = "*********"
    let prefix = "BR/ORD#"
    let suffix = "***"
    let normalized = "Widget   Red".replace("   ", " ")
    print("header=${header.starts_with(prefix)} footer=${footer.ends_with(suffix)} text=${normalized}")
end

main()
"#,
        )
        .expect("failed to write string parsing helper script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run string parsing helper script");

        assert!(
            output.status.success(),
            "string parsing helper script failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("header=true footer=true text=Widget Red"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn run_executes_std_dispatch_helper_program() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_run_std_dispatch_helpers");
        let script_path = dir.join("std_dispatch_helpers.lust");

        fs::write(
            &script_path,
            r#"
import "std/dispatch"
import "std/helpers"

fn main()
    let kind = builtin_method_kind("trim")
    print("kind=" + builtin_method_vm_name(kind))
    print("vm=" + builtin_method_vm_name(kind))
    print("arity=" + to_string(builtin_method_expected_args(kind)))
    print("call=" + infer_call_type("live"))
    print("prefers=" + to_string(pipe_prefers_method("String", "trim", false)))
    print("receiver=" + to_string(builtin_method_requires_string_receiver("trim")))
    print("builtin=" + to_string(is_builtin_method("replace")))
    print("type=" + infer_builtin_method_type("String", "split"))
    print("arg_count=" + to_string(builtin_method_arg_kinds("replace").length()))
end

main()
"#,
        )
        .expect("failed to write std dispatch helper script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run std dispatch helper script");

        assert!(
            output.status.success(),
            "std dispatch helper script failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("kind=__str_trim"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("vm=__str_trim"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("arity=0"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("call=Boolean"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("prefers=true"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("receiver=true"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("builtin=true"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("type=List<String>"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("arg_count=2"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn run_executes_std_dispatch_module_directly() {
        let root = repo_root();
        let script_path = root.join("lust_src").join("std").join("dispatch.lust");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run std dispatch module directly");

        assert!(
            output.status.success(),
            "std dispatch module direct run failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn run_executes_pipe_and_lambda_list_transform_program() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_run_pipe_lambda_transform");
        let script_path = dir.join("pipe_lambda_transform.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let raw_line = "  alpha|| beta |  "
    let clean = raw_line
        |> trim()
        |> split("|")
        |> filter(fn(x) => x.length() > 0)
        |> map(fn(x) => x.trim())
    print("len=${clean.length()} first=${clean[0]} second=${clean[1]}")
end

main()
"#,
        )
        .expect("failed to write pipe/lambda transform script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run pipe/lambda transform script");

        assert!(
            output.status.success(),
            "pipe/lambda transform script failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("len=2 first=alpha second=beta"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn run_executes_destructuring_assignment_program() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_run_destructuring_assignment");
        let script_path = dir.join("destructuring_assignment.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let [name, id] = " Jacob ,101 " |> split(",")
    let [code, _, qty] = "SKU|skip|3" |> split("|")
    print("name=${name.trim()} id=${id.trim()} code=${code} qty=${qty}")
end

main()
"#,
        )
        .expect("failed to write destructuring assignment script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run destructuring assignment script");

        assert!(
            output.status.success(),
            "destructuring assignment script failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("name=Jacob id=101 code=SKU qty=3"),
            "unexpected stdout:\n{}",
            stdout
        );
    }

    #[test]
    fn parses_trailing_rest_list_pattern() {
        let decls = parse_source(
            r#"
let [name, id, ..] = "Jacob,101,extra,data" |> split(",")
"#,
        );

        let pattern = decls
            .iter()
            .find_map(|decl| match decl {
                crate::ast::Decl::Stmt(crate::ast::Stmt::LetPattern(_, pattern, _)) => Some(pattern),
                _ => None,
            })
            .expect("destructuring let with rest not found");

        match pattern {
            crate::ast::Pattern::List(items, has_rest) => {
                assert_eq!(items.len(), 2, "unexpected item count: {:?}", items);
                assert!(*has_rest, "expected trailing rest marker");
                assert!(matches!(&items[0], crate::ast::Pattern::Bind(name) if name == "name"));
                assert!(matches!(&items[1], crate::ast::Pattern::Bind(name) if name == "id"));
            }
            other => panic!("expected list pattern, got {:?}", other),
        }
    }

    #[test]
    fn run_executes_trailing_rest_destructuring_program() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_run_destructuring_rest");
        let script_path = dir.join("destructuring_rest.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let [name, id, ..] = "Jacob,101,extra,data,here" |> split(",")
    print("name=${name} id=${id}")
end

main()
"#,
        )
        .expect("failed to write destructuring rest script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run destructuring rest script");

        assert!(
            output.status.success(),
            "destructuring rest script failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("name=Jacob id=101"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn run_executes_lustgex_extended_pattern_program() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_run_lustgex_extended");
        let script_path = dir.join("lustgex_extended.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let p1 = "start then blanks then integer then fewest anything then end"
    let p2 = "start then letter then blanks then fewest \"END\" then end"
    print(compile_lustgex(p1))
    print(compile_lustgex(p2))
end

main()
"#,
        )
        .expect("failed to write lustgex extended script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run lustgex extended script");

        assert!(
            output.status.success(),
            "lustgex extended script failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(r"^\s+\d+.*?$"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains(r"^[A-Za-z]\s+.*?END$"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn run_executes_std_lustgex_module_program() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_run_std_lustgex");
        let script_path = dir.join("std_lustgex.lust");

        fs::write(
            &script_path,
            r#"
import "std/lustgex"

fn main()
    let pattern = "start then blanks then integer then fewest anything then end"
    print(lustgex_compile(pattern))
end

main()
"#,
        )
        .expect("failed to write std lustgex script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run std lustgex script");

        assert!(
            output.status.success(),
            "std lustgex script failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(r"^\s+\d+.*?$"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn run_launch_lust_builtin_runs_repo_script() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_run_launch_builtin");
        let target = root.join("test_simple.lust");
        let script_path = dir.join("launch_builtin_run.lust");
        fs::write(
            &script_path,
            format!(
                r#"
fn main()
    let code = launch_lust("run", "{}")
    println("launcher-run-exit " + to_string(code))
end

main()
"#,
                target.display()
            ),
        )
        .expect("failed to write launch_lust run script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run launch_lust rust backend test");

        assert!(
            output.status.success(),
            "launch_lust rust backend test failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Hello from Lust!"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("launcher-run-exit 0"), "unexpected stdout:\n{}", stdout);
    }

    #[path = "compiler_tests_bridge.rs"]
    mod bridge;

    mod selfhost_validation {
        use super::*;

    #[test]
    fn selfhost_parser_eval_demo_runs_via_vm() {
        let root = repo_root();
        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                root.join("bootstrap/lust_src/selfhost_eval_demo.lust")
                    .to_str()
                    .unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust demo on vm");

        assert!(
            output.status.success(),
            "parser_in_lust vm demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 7"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_codegen_runs_via_rust_backend_on_absolute_repo_path() {
        let root = repo_root();
        let script_path = root.join("tests/spawn_test.lust");
        let _lust_cli_guard = lock_lust_cli();
        let mut command = Command::new("cargo");
        command
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ]);

        let output = run_command_with_timeout(
            command,
            Duration::from_secs(180),
            "parser_in_lust rust-codegen absolute-path run",
        );

        assert!(
            output.status.success(),
            "parser_in_lust rust-codegen absolute-path run failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("IRFn say_hi arity=1"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_codegen_handles_list_heavy_repo_script() {
        let root = repo_root();
        let script_path = root.join("tests/list_torture.lust");
        let _lust_cli_guard = lock_lust_cli();
        let mut command = Command::new("cargo");
        command
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ]);

        let output = run_command_with_timeout(
            command,
            Duration::from_secs(180),
            "parser_in_lust rust-codegen list_torture run",
        );

        assert!(
            output.status.success(),
            "parser_in_lust rust-codegen list_torture run failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("IRFn sum arity=1"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("IRList"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_codegen_handles_struct_method_repo_script() {
        let root = repo_root();
        let script_path = root.join("tests/phase3_test.lust");
        let _lust_cli_guard = lock_lust_cli();
        let mut command = Command::new("cargo");
        command
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ]);

        let output = run_command_with_timeout(
            command,
            Duration::from_secs(180),
            "parser_in_lust rust-codegen phase3_test run",
        );

        assert!(
            output.status.success(),
            "parser_in_lust rust-codegen phase3_test run failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("IRMethod Point.print_coords"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("IRStructLit Point"), "unexpected stdout:\n{}", stdout);
    }
    #[test]
    fn selfhost_parser_eval_lists_runs_via_vm() {
        let root = repo_root();
        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                "./bootstrap/lust_src/selfhost_eval_lists.lust",
            ])
            .output()
            .expect("failed to run parser_in_lust lists demo");

        assert!(
            output.status.success(),
            "parser_in_lust lists demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("big 6"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 6"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_methods_runs_via_vm() {
        let root = repo_root();
        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                "./bootstrap/lust_src/selfhost_eval_methods.lust",
            ])
            .output()
            .expect("failed to run parser_in_lust methods demo");

        assert!(
            output.status.success(),
            "parser_in_lust methods demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("outcome 5"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 5"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_stdlib_runs_via_vm() {
        let root = repo_root();
        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                "./bootstrap/lust_src/selfhost_eval_stdlib.lust",
            ])
            .output()
            .expect("failed to run parser_in_lust stdlib demo");

        assert!(
            output.status.success(),
            "parser_in_lust stdlib demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("abs(-42) = 42"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("replace = hello lust"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 42"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_file_io_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_io");
        let script_path = dir.join("selfhost_eval_io.lust");
        let output_path = dir.join("selfhost_eval_output.txt");

        fs::write(
            &script_path,
            format!(
                r#"
fn main()
    write_file("{output}", "hello capsule")
    let content = read_file("{output}")
    print(content)
    return content.length()
end
"#,
                output = output_path.display()
            ),
        )
        .expect("failed to write selfhost eval io script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust file io demo");

        assert!(
            output.status.success(),
            "parser_in_lust file io demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("hello capsule"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 13"), "unexpected stdout:\n{}", stdout);

        let written = fs::read_to_string(&output_path).expect("failed to read selfhost output file");
        assert_eq!(written, "hello capsule");
    }

    #[test]
    fn selfhost_parser_eval_json_encode_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_json");
        let script_path = dir.join("selfhost_eval_json_encode.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let encoded = json_encode("hi")
    print(encoded)
    return encoded.length()
end
"#,
        )
        .expect("failed to write selfhost eval json script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust json encode demo");

        assert!(
            output.status.success(),
            "parser_in_lust json encode demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("\"hi\""), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 4"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_json_decode_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_json");
        let script_path = dir.join("selfhost_eval_json_decode.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let decoded = json_decode("[1,2,3]")
    print(decoded)
    return decoded.length()
end
"#,
        )
        .expect("failed to write selfhost eval json decode script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust json decode demo");

        assert!(
            output.status.success(),
            "parser_in_lust json decode demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("[\n  1,\n  2,\n  3\n]"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 17"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_get_env_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_env");
        let script_path = dir.join("selfhost_eval_env.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let value = get_env("LUST_SELFHOST_VALUE")
    print(value)
    return value.length()
end
"#,
        )
        .expect("failed to write selfhost eval env script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .env("LUST_SELFHOST_VALUE", "capsule")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust get_env demo");

        assert!(
            output.status.success(),
            "parser_in_lust get_env demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("capsule"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 7"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_input_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_input");
        let script_path = dir.join("selfhost_eval_input.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let value = input()
    print("input", value)
    return value.length()
end
"#,
        )
        .expect("failed to write selfhost eval input script");

        let _lust_cli_guard = lock_lust_cli();
        let mut child = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn parser_in_lust input demo");

        {
            use std::io::Write;
            let stdin = child.stdin.as_mut().expect("missing child stdin");
            stdin
                .write_all(b"capsule\n")
                .expect("failed to write selfhost input");
        }

        let output = child
            .wait_with_output()
            .expect("failed to wait for parser_in_lust input demo");

        assert!(
            output.status.success(),
            "parser_in_lust input demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("input capsule"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 7"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_get_key_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_get_key");
        let script_path = dir.join("selfhost_eval_get_key.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let key = get_key()
    match key do
        case Up then
            print("key", "up")
            return 1
        case Char(ch) then
            print("key", ch)
            return ch.length()
        case _ then
            print("key", "other")
            return 0
    end
end
"#,
        )
        .expect("failed to write selfhost eval get_key script");

        let _lust_cli_guard = lock_lust_cli();
        let mut child = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn parser_in_lust get_key demo");

        {
            use std::io::Write;
            let stdin = child.stdin.as_mut().expect("missing child stdin");
            stdin
                .write_all(b"up\n")
                .expect("failed to write selfhost key input");
        }

        let output = child
            .wait_with_output()
            .expect("failed to wait for parser_in_lust get_key demo");

        assert!(
            output.status.success(),
            "parser_in_lust get_key demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("key up"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 1"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_pattern_guard_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_guard");
        let script_path = dir.join("selfhost_eval_guard.lust");

        fs::write(
            &script_path,
            r#"
enum Step = Add(n) | Stop

fn main()
    match Add(5) do
        case Add(n) if n > 3 then
            print("guard hit", n)
            return n
        case Add(n) then
            return 0
        case Stop then
            return -1
    end
end
"#,
        )
        .expect("failed to write selfhost eval guard script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust guard demo");

        assert!(
            output.status.success(),
            "parser_in_lust guard demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("guard hit 5"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 5"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_struct_pattern_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_struct_pattern");
        let script_path = dir.join("selfhost_eval_struct_pattern.lust");

        fs::write(
            &script_path,
            r#"
type User = { name, age }

fn main()
    let user = User { name: "Kid", age: 17 }
    match user do
        case User { name: "Admin" } then
            return -1
        case User { age: a } if a < 18 then
            print("young", a)
            return a
    end
    return 0
end
"#,
        )
        .expect("failed to write selfhost eval struct pattern script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust struct pattern demo");

        assert!(
            output.status.success(),
            "parser_in_lust struct pattern demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("young 17"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 17"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_nested_enum_struct_pattern_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_nested_pattern");
        let script_path = dir.join("selfhost_eval_nested_pattern.lust");

        fs::write(
            &script_path,
            r#"
type User = { name, age }
enum Message = Joined(user) | Quit

fn main()
    let msg = Joined(User { name: "Ana", age: 20 })
    match msg do
        case Joined(User { name: name }) then
            print("joined", name)
            return name.length()
        case Quit then
            return -1
    end
    return 0
end
"#,
        )
        .expect("failed to write selfhost eval nested pattern script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust nested pattern demo");

        assert!(
            output.status.success(),
            "parser_in_lust nested pattern demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("joined Ana"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 3"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_nested_guard_and_fallback_patterns_run_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_nested_guard_pattern");
        let script_path = dir.join("selfhost_eval_nested_guard_pattern.lust");

        fs::write(
            &script_path,
            r#"
type User = { name, age }
enum Message = Joined(user) | Quit

fn main()
    let msg = Joined(User { name: "Mia", age: 17 })
    match msg do
        case Joined(User { age: age }) if age > 18 then
            return -1
        case Joined(User { name: name, age: age }) if age < 18 then
            print("guarded", name, age)
            return age
        case _ then
            return 0
    end
end
"#,
        )
        .expect("failed to write selfhost eval nested guard pattern script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust nested guard pattern demo");

        assert!(
            output.status.success(),
            "parser_in_lust nested guard pattern demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("guarded Mia 17"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 17"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_wildcard_heavy_nested_patterns_run_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_wildcard_nested_pattern");
        let script_path = dir.join("selfhost_eval_wildcard_nested_pattern.lust");

        fs::write(
            &script_path,
            r#"
type Profile = { name, age }
type User = { profile, role }
enum Message = Joined(user) | Quit

fn main()
    let msg = Joined(User { profile: Profile { name: "Zoe", age: 22 }, role: "admin" })
    match msg do
        case Joined(User { profile: Profile { name: _, age: age }, role: _ }) if age > 20 then
            print("wild-nested", age)
            return age
        case _ then
            return 0
    end
end
"#,
        )
        .expect("failed to write selfhost eval wildcard nested pattern script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust wildcard nested pattern demo");

        assert!(
            output.status.success(),
            "parser_in_lust wildcard nested pattern demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("wild-nested 22"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 22"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_deeper_enum_struct_nesting_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_deeper_nesting");
        let script_path = dir.join("selfhost_eval_deeper_nesting.lust");

        fs::write(
            &script_path,
            r#"
type Profile = { name, age }
type User = { profile }
type Envelope = { msg }
enum Message = Joined(user) | Quit
enum Outer = Wrap(env) | Empty

fn main()
    let outer = Wrap(Envelope { msg: Joined(User { profile: Profile { name: "Ivy", age: 29 } }) })
    match outer do
        case Wrap(Envelope { msg: Joined(User { profile: Profile { name: name, age: age } }) }) then
            print("deep", name, age)
            return age
        case Empty then
            return -1
        case _ then
            return 0
    end
end
"#,
        )
        .expect("failed to write selfhost eval deeper nesting script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust deeper nesting demo");

        assert!(
            output.status.success(),
            "parser_in_lust deeper nesting demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("deep Ivy 29"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 29"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_pattern_arity_mismatch_falls_through_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_pattern_arity");
        let script_path = dir.join("selfhost_eval_pattern_arity.lust");

        fs::write(
            &script_path,
            r#"
enum Msg = Pair(a, b) | One(n)

fn main()
    let msg = One(7)
    match msg do
        case One(left, right) then
            return 99
        case One(value) then
            print("arity-fallback", value)
            return value
        case _ then
            return -1
    end
end
"#,
        )
        .expect("failed to write selfhost eval pattern arity script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust pattern arity demo");

        assert!(
            output.status.success(),
            "parser_in_lust pattern arity demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("arity-fallback 7"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 7"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_bool_and_null_patterns_run_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_literal_patterns");
        let script_path = dir.join("selfhost_eval_literal_patterns.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let truthy = false
    let maybe = null
    let score = 0

    match truthy do
        case true then
            return -1
        case false then
            score = score + 2
    end

    match maybe do
        case null then
            score = score + 3
        case _ then
            return -2
    end

    print("literal-patterns", score)
    return score
end
"#,
        )
        .expect("failed to write selfhost eval literal pattern script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust literal pattern demo");

        assert!(
            output.status.success(),
            "parser_in_lust literal pattern demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("literal-patterns 5"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 5"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_runtime_integration_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_integration");
        let script_path = dir.join("selfhost_eval_integration.lust");
        let output_path = dir.join("integration_output.txt");

        fs::write(
            &script_path,
            format!(
                r#"
enum Status = Ok(value) | Fail

fn classify(n)
    if n > 10 then
        return Ok(n)
    else
        return Fail
    end
end

fn main()
    let env_name = get_env("LUST_SELFHOST_NAME")
    let encoded = json_encode(env_name)
    let decoded = json_decode("[1,2,3]")
    write_file("{output}", "name=" + env_name + ";encoded=" + encoded)
    let saved = read_file("{output}")
    match classify(saved.length()) do
        case Ok(n) if n > 10 then
            print("integration", env_name, encoded)
            print(decoded)
            print(saved)
            return n
        case Ok(n) then
            return 0
        case Fail then
            return -1
    end
end
"#,
                output = output_path.display()
            ),
        )
        .expect("failed to write selfhost integration script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .env("LUST_SELFHOST_NAME", "capsule")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust integration demo");

        assert!(
            output.status.success(),
            "parser_in_lust integration demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("integration capsule \"capsule\""), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("[\n  1,\n  2,\n  3\n]"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("name=capsule;encoded=\"capsule\""), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 30"), "unexpected stdout:\n{}", stdout);

        let written = fs::read_to_string(&output_path).expect("failed to read integration output file");
        assert_eq!(written, "name=capsule;encoded=\"capsule\"");
    }

    #[test]
    fn selfhost_parser_eval_broader_real_program_flow_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_real_program");
        let script_path = dir.join("selfhost_eval_real_program.lust");
        let output_path = dir.join("real_program_output.txt");

        fs::write(
            &script_path,
            r#"
type Profile = { name, role }
type User = { profile, tags }
enum Task = Save(user, path) | Skip(reason)
enum Result = Stored(path, size) | Rejected(reason)

fn build_task(args, env_name)
    if args.length() > 1 and env_name != "" then
        return Save(
            User {
                profile: Profile { name: env_name.trim(), role: "builder".replace("builder", "builder") },
                tags: json_decode("[\"lust\",\"selfhost\"]")
            },
            args[1]
        )
    end
    return Skip("missing-input")
end

fn run_task(task)
    match task do
        case Save(User { profile: Profile { name: name, role: role }, tags: tags }, path) then
            if tags.contains("lust") and tags.contains("selfhost") then
                let summary = "name=" + name + ";role=" + role + ";tags=ok"
                write_file(path, summary)
                let saved = read_file(path)
                if saved.contains(name) and saved.contains(role) then
                    return Stored(path, saved.length())
                end
            end
            return Rejected("verification-failed")
        case Skip(reason) then
            return Rejected(reason)
    end
end

fn main()
    let args = get_args()
    let env_name = get_env("LUST_REAL_FLOW_NAME")
    let encoded = json_encode(env_name.trim())
            let result = run_task(build_task(args, env_name))

    match result do
        case Stored(path, size) if size > 10 then
            print("real-flow", path, encoded, size)
            return size
        case Stored(path, size) then
            return 0
        case Rejected(reason) then
            print("real-flow-rejected", reason)
            return -1
    end
end
"#,
        )
        .expect("failed to write selfhost real program script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .env("LUST_REAL_FLOW_NAME", " capsule ")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
                output_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust real program demo");

        assert!(
            output.status.success(),
            "parser_in_lust real program demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(
            stdout.contains(&format!("real-flow {} \"capsule\"", output_path.display())),
            "unexpected stdout:\n{}",
            stdout
        );
        assert!(stdout.contains("EVAL main 33"), "unexpected stdout:\n{}", stdout);

        let written = fs::read_to_string(&output_path).expect("failed to read real program output file");
        assert_eq!(written, "name=capsule;role=builder;tags=ok");
    }

    #[test]
    fn selfhost_parser_eval_draw_runtime_stubs_run_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_draw_runtime");
        let script_path = dir.join("selfhost_eval_draw_runtime.lust");

        fs::write(
            &script_path,
            r#"
import "draw"

fn main()
    print(live(), dark_gray)
    window(64, 64, "draw smoke")
    clear_screen(neon_pink)
    circle(32, 32, 8, blue)
    rect(2, 40, 12, 8, green)
    line(0, 63, 63, 0, red)
    triangle(48, 8, 60, 24, 40, 24, white)
    text("ok", 4, 4, 8, black)
    return 0
end

main()
"#,
        )
        .expect("failed to write selfhost draw runtime script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust draw runtime demo");

        assert!(
            output.status.success(),
            "parser_in_lust draw runtime demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("0 dark_gray"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL top null"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_sleep_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_sleep");
        let script_path = dir.join("selfhost_eval_sleep.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    sleep(0)
    print("slept")
    return 1
end
"#,
        )
        .expect("failed to write selfhost sleep script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust sleep demo");

        assert!(
            output.status.success(),
            "parser_in_lust sleep demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("slept"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 1"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_debug_and_assert_run_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_debug");
        let script_path = dir.join("selfhost_eval_debug.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let seen = debug("selfhost", 9)
    assert(seen == 9, "debug should pass value through")
    print("debug-ok")
    return seen
end
"#,
        )
        .expect("failed to write selfhost debug script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser.lust debug demo");

        assert!(
            output.status.success(),
            "parser.lust debug demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("DEBUG selfhost 9"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("debug-ok"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 9"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_get_args_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_args");
        let script_path = dir.join("selfhost_eval_args.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let args = get_args()
    print(args.length(), args[0], args[1], args[2])
    return args.length()
end
"#,
        )
        .expect("failed to write selfhost eval get_args script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
                "alpha",
                "beta",
            ])
            .output()
            .expect("failed to run parser_in_lust get_args demo");

        assert!(
            output.status.success(),
            "parser_in_lust get_args demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let script_display = script_path.display().to_string();
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(
            stdout.contains(&format!("3 {} alpha beta", script_display)),
            "unexpected stdout:\n{}",
            stdout
        );
        assert!(stdout.contains("EVAL main 3"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_eval_type_of_runs_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_type_of");
        let script_path = dir.join("selfhost_eval_type_of.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    print(type_of("hi"), type_of(7), type_of(null))
    return 1
end
"#,
        )
        .expect("failed to write selfhost eval type_of script");

        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser.lust type_of demo");

        assert!(
            output.status.success(),
            "parser.lust type_of demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("VALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("String Number Null"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("EVAL main 1"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_executes_break_via_vm() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_break_unclaimed");
        let script_path = dir.join("selfhost_eval_break_unclaimed.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let i = 0
    while i < 3 do
        break
    end
end
"#,
        )
        .expect("failed to write selfhost eval break script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust break boundary demo");

        assert!(
            output.status.success(),
            "parser_in_lust break boundary demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("VALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("INVALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(
            stdout.contains("identifier is not declared break in main"),
            "unexpected stdout:\n{}",
            stdout
        );
        assert!(stdout.contains("EVAL main unsupported"), "unexpected stdout:\n{}", stdout);
    }

    #[test]
    fn selfhost_parser_keeps_list_patterns_unclaimed_for_now() {
        let root = repo_root();
        let dir = fresh_temp_dir("lust_selfhost_eval_list_pattern_unclaimed");
        let script_path = dir.join("selfhost_eval_list_pattern_unclaimed.lust");

        fs::write(
            &script_path,
            r#"
fn main()
    let values = [1, 2]
    match values do
        case [1, second] then
            return second
        case _ then
            return 0
    end
end
"#,
        )
        .expect("failed to write selfhost eval list pattern script");

        let _lust_cli_guard = lock_lust_cli();
        let output = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", "/tmp/lust-target")
            .args([
                "run",
                "--quiet",
                "--bin",
                "lust",
                "--",
                "run",
                "lust_src/parser.lust",
                script_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run parser_in_lust list-pattern boundary demo");

        assert!(
            output.status.success(),
            "parser_in_lust list-pattern boundary demo failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("expected Then, got Number"),
            "unexpected stdout:\n{}",
            stdout
        );
        assert!(stdout.contains("INVALID ast"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("unexpected PatternError node"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("INVALID symbols"), "unexpected stdout:\n{}", stdout);
        assert!(stdout.contains("IRPatternUnsupported PatternError"), "unexpected stdout:\n{}", stdout);
    }

    }

    #[test]
    fn vm_executes_get_args_builtin() {
        let vm = run_vm_snippet_with_args(
            r#"
let args = get_args()
print(args.length(), args[0], args[1])
"#,
            vec!["alpha".to_string(), "beta".to_string()],
        );

        assert_eq!(vm.output(), &["2 alpha beta".to_string()]);
    }

    #[test]
    fn vm_executes_sleep_builtin() {
        let vm = run_vm_snippet(
            r#"
sleep(0)
print("slept")
"#,
        );

        assert_eq!(vm.output(), &["slept".to_string()]);
    }

    #[test]
    fn vm_executes_launch_lust_builtin() {
        let target = repo_root().join("test_simple.lust");
        let _lust_cli_guard = lock_lust_cli();
        let vm = run_vm_snippet(&format!(
            r#"
let code = launch_lust("run", "{}")
print("launcher-exit", code)
"#,
            target.display()
        ));

        assert_eq!(vm.output(), &["launcher-exit 0".to_string()]);
    }

    #[test]
    fn vm_accepts_spawn_as_current_noop_semantics() {
        let vm = run_vm_snippet(
            r#"
fn say_hi(name)
    print("Hello from thread,", name)
end

spawn say_hi("Alice")
print("main")
"#,
        );

        assert_eq!(vm.output(), &["main".to_string()]);
    }

    #[test]
    fn vm_accepts_import_audio_and_calls_audio_builtin() {
        let _vm = run_vm_snippet(
            r#"
import "audio"
audio_note_off()
"#,
        );
    }

    #[test]
    fn vm_accepts_import_draw_and_exposes_draw_builtins() {
        let vm = run_vm_snippet(
            r#"
import "draw"
print(live(), dark_gray)
window(800, 600, "demo")
clear_screen(neon_pink)
circle(10, 20, 3, red)
rect(1, 2, 3, 4, blue)
line(0, 0, 5, 5, green)
triangle(0, 0, 3, 0, 1, 4, white)
"#,
        );

        assert_eq!(vm.output(), &["false dark_gray".to_string()]);
    }

    #[test]
    fn vm_rejects_audio_builtin_without_import() {
        let mut lexer = Lexer::new(
            r#"
audio_note_off()
"#,
        );
        let mut tokens = Vec::new();
        while let Some(t) = lexer.next_token() {
            tokens.push(t);
        }
        let mut parser = Parser::new(tokens);
        let decls = parser.parse();
        assert!(parser.errors.is_empty(), "parse errors: {:?}", parser.errors);
        let type_info = TypeChecker::new()
            .check(&decls)
            .expect("typecheck failed");
        let program = BytecodeCompiler::new(type_info)
            .compile(&decls)
            .expect("bytecode compile failed");
        let mut vm = Vm::new_with_args(program, vec![]);
        let err = vm.run().expect_err("expected missing import error");
        assert!(err.contains("requires `import \"audio\"`"));
    }

    #[test]
    fn vm_executes_string_methods_needed_for_parser_code() {
        let vm = run_vm_snippet(
            r#"
let s = "  abc,def  ".trim()
let parts = s.split(",")
let chars = "xy".to_list()
let lines = "a\nb\nc".lines()
let text = "Widget   Red".replace("   ", " ")
print(parts[0], parts[1], "abc".at(1), "abcdef".slice(1, 4), "hello".contains("ell"), chars.length(), "header".starts_with("he"), "footer".ends_with("er"), lines.length(), text)
"#,
        );

        assert_eq!(vm.output(), &["abc def b bcd true 2 true true 3 Widget Red".to_string()]);
    }

    #[test]
    fn vm_executes_pipe_dispatch_via_shared_semantics() {
        let vm = run_vm_snippet(
            r#"
fn add_prefix(name: String, prefix: String) -> String
    return prefix + name
end

let label = "Ada" |> add_prefix("Dr. ")
let trimmed = "  hello  " |> trim()
print(label, trimmed)
"#,
        );

        assert_eq!(vm.output(), &["Dr. Ada hello".to_string()]);
    }

    #[test]
    fn vm_executes_slice_expressions_via_shared_access_lowering() {
        let vm = run_vm_snippet(
            r#"
let text = "ORD20260327_USR99"
let prefix = text[0..3]
let date = text[3..11]
let user = text[12..]
let all = text[..]
let items = [10, 20, 30, 40]
let middle = items[1..3]
print(prefix, date, user, all, middle[0], middle[1], middle.length())
"#,
        );

        assert_eq!(
            vm.output(),
            &["ORD 20260327 USR99 ORD20260327_USR99 20 30 2".to_string()]
        );
    }

    #[test]
    fn rejects_non_exhaustive_match_without_fallback() {
        let mut lexer = Lexer::new(
            r#"
type User = { name, age }

fn main()
    let user = User { name: "Admin", age: 30 }
    match user do
        case User { name: "Admin" } then
            print("boss")
    end
end
"#,
        );
        let mut tokens = Vec::new();
        while let Some(t) = lexer.next_token() {
            tokens.push(t);
        }
        let mut parser = Parser::new(tokens);
        let decls = parser.parse();
        assert!(parser.errors.is_empty(), "parse errors: {:?}", parser.errors);
        let errs = TypeChecker::new().check(&decls).expect_err("expected match exhaustiveness error");
        assert!(errs.iter().any(|e| e.contains("non-exhaustive match")));
    }

    #[test]
    fn accepts_exhaustive_struct_match_without_fallback() {
        let decls = parse_snippet(
            r#"
type User = { name, age }

fn main()
    let user = User { name: "Admin", age: 30 }
    match user do
        case User { age: a } then
            print(a)
    end
end
"#,
        );

        TypeChecker::new()
            .check(&decls)
            .expect("struct-typed match should be treated as exhaustive");
    }

    #[test]
    fn accepts_exhaustive_enum_match_inferred_from_case_variants() {
        let decls = parse_snippet(
            r#"
enum Option = Some(value) | None

fn show(opt)
    match opt do
        case Some(v) then
            print(v)
        case None then
            print("none")
    end
end
"#,
        );

        TypeChecker::new()
            .check(&decls)
            .expect("enum coverage should be inferred from unguarded case variants");
    }

    #[test]
    fn rejects_unreachable_case_after_total_match_case() {
        let mut lexer = Lexer::new(
            r#"
fn main()
    let x = 1
    match x do
        case a then
            print(a)
        case _ then
            print("never")
    end
end
"#,
        );
        let mut tokens = Vec::new();
        while let Some(t) = lexer.next_token() {
            tokens.push(t);
        }
        let mut parser = Parser::new(tokens);
        let decls = parser.parse();
        assert!(parser.errors.is_empty(), "parse errors: {:?}", parser.errors);
        let errs = TypeChecker::new().check(&decls).expect_err("expected unreachable match case error");
        assert!(errs.iter().any(|e| e.contains("unreachable match case")));
    }

    #[test]
    fn rejects_break_outside_while_loop() {
        let decls = parse_snippet(
            r#"
fn main()
    break
end
"#,
        );

        let errs = TypeChecker::new().check(&decls).expect_err("expected break misuse error");
        assert!(errs.iter().any(|e| e.contains("break is only valid inside a while loop")));
    }

    #[test]
    fn rejects_continue_outside_while_loop() {
        let decls = parse_snippet(
            r#"
fn main()
    if true then
        continue
    end
end
"#,
        );

        let errs = TypeChecker::new().check(&decls).expect_err("expected continue misuse error");
        assert!(errs.iter().any(|e| e.contains("continue is only valid inside a while loop")));
    }

    #[test]
    fn rejects_return_outside_function() {
        let decls = parse_snippet(
            r#"
return 1
"#,
        );

        let errs = TypeChecker::new().check(&decls).expect_err("expected return misuse error");
        assert!(errs.iter().any(|e| e.contains("return is only valid inside a function")));
    }

    #[test]
    fn rejects_typed_let_initializer_mismatch() {
        let mut lexer = Lexer::new(
            r#"
fn main()
    let x: Number = "hello"
end
"#,
        );
        let mut tokens = Vec::new();
        while let Some(t) = lexer.next_token() {
            tokens.push(t);
        }
        let mut parser = Parser::new(tokens);
        let decls = parser.parse();
        assert!(parser.errors.is_empty(), "parse errors: {:?}", parser.errors);
        let errs = TypeChecker::new().check(&decls).expect_err("expected typed let mismatch");
        assert!(errs.iter().any(|e| e.contains("initializer for 'x'") && e.contains("expected Number") && e.contains("got String")));
    }

    #[test]
    fn rejects_typed_struct_field_mismatch() {
        let mut lexer = Lexer::new(
            r#"
type User = { age: Number }

fn main()
    let user = User { age: "old" }
end
"#,
        );
        let mut tokens = Vec::new();
        while let Some(t) = lexer.next_token() {
            tokens.push(t);
        }
        let mut parser = Parser::new(tokens);
        let decls = parser.parse();
        assert!(parser.errors.is_empty(), "parse errors: {:?}", parser.errors);
        let errs = TypeChecker::new().check(&decls).expect_err("expected struct field mismatch");
        assert!(errs.iter().any(|e| e.contains("field 'User.age'") && e.contains("expected Number") && e.contains("got String")));
    }

    #[test]
    fn rejects_typed_function_argument_mismatch() {
        let mut lexer = Lexer::new(
            r#"
fn add(a: Number) -> Number
    return a
end

fn main()
    let x = add("oops")
end
"#,
        );
        let mut tokens = Vec::new();
        while let Some(t) = lexer.next_token() {
            tokens.push(t);
        }
        let mut parser = Parser::new(tokens);
        let decls = parser.parse();
        assert!(parser.errors.is_empty(), "parse errors: {:?}", parser.errors);
        let errs = TypeChecker::new().check(&decls).expect_err("expected function argument mismatch");
        assert!(errs.iter().any(|e| e.contains("argument 1 in call to 'add'") && e.contains("expected Number") && e.contains("got String")));
    }

    #[test]
    fn rejects_non_exhaustive_enum_match_without_all_variants() {
        let mut lexer = Lexer::new(
            r#"
enum Option = Some(value) | None

fn main()
    match Some(1) do
        case Some(v) then
            print(v)
    end
end
"#,
        );
        let mut tokens = Vec::new();
        while let Some(t) = lexer.next_token() {
            tokens.push(t);
        }
        let mut parser = Parser::new(tokens);
        let decls = parser.parse();
        assert!(parser.errors.is_empty(), "parse errors: {:?}", parser.errors);
        let errs = TypeChecker::new().check(&decls).expect_err("expected enum exhaustiveness error");
        assert!(errs.iter().any(|e| e.contains("non-exhaustive match on enum")));
    }

}
