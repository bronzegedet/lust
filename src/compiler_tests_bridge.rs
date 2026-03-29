use super::*;

#[test]
fn lust_side_typechecker_slice_runs_via_vm() {
    let root = repo_root();
    let _lust_cli_guard = lock_lust_cli();
    let mut command = lust_cli_command();
    let output = command
        .current_dir(&root)
        .args(["run", "lust_src/typecheck.lust"])
        .output()
        .expect("failed to run lust-side typechecker slice via vm");

    assert!(
        output.status.success(),
        "lust-side typechecker slice vm run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(normalized.contains("declared User Key List<String>"), "unexpected stdout:\n{}", stdout);
    assert!(normalized.contains("compat 1 0"), "unexpected stdout:\n{}", stdout);
    assert!(normalized.contains("unify List<String> Dynamic"), "unexpected stdout:\n{}", stdout);
    assert!(normalized.contains("declared-check ok"), "unexpected stdout:\n{}", stdout);
    assert!(
        normalized.contains("Lust Error on line 18: type mismatch in initializer for 'count': expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(normalized.contains("explicit-dynamic ok"), "unexpected stdout:\n{}", stdout);
    assert!(normalized.contains("field-errors 1"), "unexpected stdout:\n{}", stdout);
    assert!(
        normalized.contains("Lust Error on line 24: type mismatch in field 'User.score': expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(normalized.contains("fn-errors 1"), "unexpected stdout:\n{}", stdout);
    assert!(
        normalized.contains("Lust Error on line 31: type mismatch in parameter 'role' in fn 'build_user': expected String, got Number"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("struct-bind user_name String user_score Number"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("enum-bind payload User"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("match-exhaustive 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("match-missing 1 Lust Error on line 48: non-exhaustive match on enum Result; missing variants or `_` fallback"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("match-unreachable 1 Lust Error on line 54: unreachable match case after a total case"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("let-errors 1 Lust Error on line 60: type mismatch in initializer for 'count': expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("env-lookup Number String Result"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("env-merge Number String Result"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("assign-error Lust Error on line 66: type mismatch in assignment to 'count': expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(normalized.contains("assign-ok"), "unexpected stdout:\n{}", stdout);
    assert!(
        normalized.contains("branch-merge Number"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("flow-binding Number"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("flow-errors 2 Lust Error on line 73: type mismatch in assignment to 'total': expected Number, got String Lust Error on line 75: unknown identifier 'missing'"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("if-merge String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("if-errors 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("pattern-flow-bind String Number"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("pattern-flow-errors 1 Lust Error on line 90: unknown identifier 'missing_pattern_name'"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("return-errors 1 Lust Error on line 94: type mismatch in return value: expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("fn-body-bind String Number String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("fn-body-errors 1 Lust Error on line 102: type mismatch in assignment to 'count': expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("fn-if-bind Number"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("fn-if-errors 1 Lust Error on line 109: type mismatch in return value: expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("synthetic-fn accumulate seed:Number,total:Number 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("synthetic-fn-if choose_count count:Number 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("synthetic-fn-errors broken_total seed:Number 3 Lust Error on line 124: type mismatch in assignment to 'seed': expected Number, got String | Lust Error on line 125: unknown identifier 'missing_total' | Lust Error on line 126: type mismatch in return value: expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("synthetic-program synthetic-program 3 accumulate [seed:Number,total:Number] errors=0 || choose_count [count:Number] errors=0 || broken_total [seed:Number] errors=3 Lust Error on line 124: type mismatch in assignment to 'seed': expected Number, got String | Lust Error on line 125: unknown identifier 'missing_total' | Lust Error on line 126: type mismatch in return value: expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("synthetic-module core_math 3 accumulate [seed:Number,total:Number] errors=0 || choose_count [count:Number] errors=0 || broken_total [seed:Number] errors=3 Lust Error on line 124: type mismatch in assignment to 'seed': expected Number, got String | Lust Error on line 125: unknown identifier 'missing_total' | Lust Error on line 126: type mismatch in return value: expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("duplicate-fn dup_names x:Number,x:Number,tmp:Number,tmp:Number 2 Lust Error on line 0: duplicate parameter name 'x' | Lust Error on line 0: duplicate local name 'tmp'"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("collision-fn shadow_param count:Number,count:Number 1 Lust Error on line 0: local name 'count' collides with parameter name"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("duplicate-pattern-fn Number 1 Lust Error on line 141: duplicate pattern binding name 'value'"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("collision-pattern-fn Number 1 Lust Error on line 145: pattern binding 'seed' collides with existing scope name"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(normalized.contains("call-ok 0"), "unexpected stdout:\n{}", stdout);
    assert!(
        normalized.contains("call-bad 2 Lust Error on line 151: type mismatch in argument 1 in call to 'build_user': expected String, got Number Lust Error on line 151: type mismatch in argument 2 in call to 'build_user': expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("call-arity 1 Lust Error on line 152: arity mismatch in call to 'build_user': expected 2, got 1"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("call-flow String 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(normalized.contains("method-ok 0"), "unexpected stdout:\n{}", stdout);
    assert!(
        normalized.contains("method-bad-receiver 1 Lust Error on line 155: type mismatch in receiver in method call 'user.rename': expected User, got Result"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("method-bad-arg 1 Lust Error on line 156: type mismatch in argument 1 in call to 'rename': expected String, got Number"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("method-flow String 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(normalized.contains("struct-lit-ok 0"), "unexpected stdout:\n{}", stdout);
    assert!(
        normalized.contains("struct-lit-bad 2 Lust Error on line 161: type mismatch in field 'User.name' in literal: expected String, got Number Lust Error on line 161: type mismatch in field 'User.score' in literal: expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("struct-lit-arity 1 Lust Error on line 162: arity mismatch in struct literal 'User'"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(normalized.contains("enum-ctor-ok 0"), "unexpected stdout:\n{}", stdout);
    assert!(
        normalized.contains("enum-ctor-bad 1 Lust Error on line 165: type mismatch in payload 0 in constructor 'Result.Ok': expected User, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("enum-ctor-arity 1 Lust Error on line 166: arity mismatch in constructor 'Result.Pair': expected 2, got 1"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(normalized.contains("bin-ok"), "unexpected stdout:\n{}", stdout);
    assert!(
        normalized.contains("bin-bad Lust Error on line 169: type mismatch in left operand of '+': expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(normalized.contains("eq-ok"), "unexpected stdout:\n{}", stdout);
    assert!(
        normalized.contains("eq-bad Lust Error on line 171: incompatible operands for '==': User and String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(normalized.contains("cmp-ok"), "unexpected stdout:\n{}", stdout);
    assert!(
        normalized.contains("cmp-bad Lust Error on line 173: type mismatch in left operand of '>': expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(normalized.contains("cond-ok"), "unexpected stdout:\n{}", stdout);
    assert!(
        normalized.contains("cond-bad Lust Error on line 175: type mismatch in condition: expected Boolean, got Number"),
        "unexpected stdout:\n{}",
        stdout
    );
}

#[test]
fn parsed_function_shape_matches_lust_side_typechecker_bridge_assumptions() {
    let decls = parse_source(
        r#"
fn bridge_demo(name: String, count: Number) -> Number
let label: String = name
if count > 0 then
    return count
else
    return 0
end
end
"#,
    );

    let (params, ret_type, body) = decls
        .iter()
        .find_map(|decl| match decl {
            crate::ast::Decl::Fn(name, None, params, ret_type, body) if name == "bridge_demo" => {
                Some((params.clone(), ret_type.clone(), body.clone()))
            }
            _ => None,
        })
        .expect("bridge_demo function not found");

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

    assert_eq!(param_names, vec!["name".to_string(), "count".to_string()]);
    assert_eq!(param_types, vec!["String".to_string(), "Number".to_string()]);
    assert_eq!(ret_type.as_deref(), Some("Number"));
    assert_eq!(stmt_kinds, vec!["let", "if"]);
    assert_eq!(stmt_lines, vec![3, 4]);
    assert_eq!(stmt_names, vec!["label".to_string(), String::new()]);
    assert_eq!(declared_types, vec!["String".to_string(), String::new()]);
    assert_eq!(actual_types, vec!["String".to_string(), String::new()]);
    assert_eq!(
        expr_shapes,
        vec![
            "ident:name".to_string(),
            "binary(ident:count > Number)".to_string()
        ]
    );

    match &body[1] {
        crate::ast::Stmt::If(_, cond, then_body, Some(else_body)) => {
            assert!(matches!(
                cond,
                crate::ast::Expr::Binary(left, op, right)
                if matches!(&**left, crate::ast::Expr::Ident(name) if name == "count")
                    && op == ">"
                    && matches!(&**right, crate::ast::Expr::Number(n) if (*n - 0.0).abs() < f64::EPSILON)
            ));
            assert!(matches!(
                then_body.as_slice(),
                [crate::ast::Stmt::Return(_, crate::ast::Expr::Ident(name))] if name == "count"
            ));
            assert!(matches!(
                else_body.as_slice(),
                [crate::ast::Stmt::Return(_, crate::ast::Expr::Number(n))] if (*n - 0.0).abs() < f64::EPSILON
            ));
        }
        other => panic!("unexpected second stmt: {:?}", other),
    }
}

#[test]
fn parsed_richer_function_shape_matches_lust_side_typechecker_bridge_assumptions() {
    let decls = parse_source(
        r#"
type User = { name, score }
enum Result = Ok(User) | Err(String)

fn build_user(name: String, score: Number) -> User
let user: User = User { name: name, score: score }
let renamed: String = user.rename("neo")
let wrapped: Result = Ok(user)
let cloned: User = build_user(name, score)
if score > 0 then
    return build_user(name, score)
else
    return user
end
end
"#,
    );

    let (params, ret_type, body) = decls
        .iter()
        .find_map(|decl| match decl {
            crate::ast::Decl::Fn(name, None, params, ret_type, body) if name == "build_user" => {
                Some((params.clone(), ret_type.clone(), body.clone()))
            }
            _ => None,
        })
        .expect("build_user function not found");

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

    assert_eq!(param_names, vec!["name".to_string(), "score".to_string()]);
    assert_eq!(param_types, vec!["String".to_string(), "Number".to_string()]);
    assert_eq!(ret_type.as_deref(), Some("User"));
    assert_eq!(stmt_kinds, vec!["let", "let", "let", "let", "if"]);
    assert_eq!(stmt_lines, vec![6, 7, 8, 9, 10]);
    assert_eq!(
        stmt_names,
        vec!["user".to_string(), "renamed".to_string(), "wrapped".to_string(), "cloned".to_string(), String::new()]
    );
    assert_eq!(
        declared_types,
        vec!["User".to_string(), "String".to_string(), "Result".to_string(), "User".to_string(), String::new()]
    );
    assert_eq!(
        actual_types,
        vec!["Dynamic".to_string(), "Dynamic".to_string(), "Dynamic".to_string(), "Dynamic".to_string(), String::new()]
    );
    assert_eq!(
        expr_shapes,
        vec![
            "struct(User;name:ident:name,score:ident:score)".to_string(),
            "method(ident:user.rename;String)".to_string(),
            "enum(Ok;ident:user)".to_string(),
            "call(build_user;ident:name,ident:score)".to_string(),
            "binary(ident:score > Number)".to_string(),
        ]
    );

    match &body[4] {
        crate::ast::Stmt::If(_, _, then_body, Some(else_body)) => {
            assert!(matches!(
                then_body.as_slice(),
                [crate::ast::Stmt::Return(_, crate::ast::Expr::Call(name, args))]
                    if name == "build_user"
                        && matches!(args.as_slice(), [crate::ast::Expr::Ident(a), crate::ast::Expr::Ident(b)] if a == "name" && b == "score")
            ));
            assert!(matches!(
                else_body.as_slice(),
                [crate::ast::Stmt::Return(_, crate::ast::Expr::Ident(name))] if name == "user"
            ));
        }
        other => panic!("unexpected fourth stmt: {:?}", other),
    }
}

#[test]
fn parsed_function_shape_runs_through_lust_typechecker_bridge_mode() {
    let bridge = reduced_bridge_fn_from_source(
        r#"
fn bridge_demo(name: String, count: Number) -> Number
let label: String = name
if count > 0 then
    return count
else
    return 0
end
end
"#,
        "bridge_demo",
    );

    let output = run_reduced_bridge_fn(&bridge);

    assert!(
        output.status.success(),
        "lust-side typechecker bridge mode failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("bridge-fn bridge_demo name:String,count:Number,label:String 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-exprs 2 ident:name"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-exprs-tail binary(ident:count > Number) binary(ident:count > Number)"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-binary 1"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-struct 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-method 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-enum 0"),
        "unexpected stdout:\n{}",
        stdout
    );
}

#[test]
fn parsed_richer_function_shape_runs_through_lust_typechecker_bridge_mode() {
    let bridge = reduced_bridge_fn_from_source(
        r#"
type User = { name, score }
enum Result = Ok(User) | Err(String)

fn build_user(name: String, score: Number) -> User
let user: User = User { name: name, score: score }
let renamed: String = user.rename("neo")
let wrapped: Result = Ok(user)
let cloned: User = build_user(name, score)
if score > 0 then
    return build_user(name, score)
else
    return user
end
end
"#,
        "build_user",
    );

    let output = run_reduced_bridge_fn(&bridge);

    assert!(
        output.status.success(),
        "lust-side richer bridge mode failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("bridge-fn build_user name:String,score:Number,user:User,renamed:String,wrapped:Result,cloned:User 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-exprs 5 struct(User;name:ident:name,score:ident:score)"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-exprs-tail method(ident:user.rename;String) binary(ident:score > Number)"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-binary 1"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-struct 1"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-method 1"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-call 1"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-enum 1"),
        "unexpected stdout:\n{}",
        stdout
    );
}

#[test]
fn parsed_list_function_shape_runs_through_lust_typechecker_bridge_mode() {
    let bridge = reduced_bridge_fn_from_source(
        r#"
fn build_list_count() -> Number
let names: List<String> = ["ana", "bob"]
return 2
end
"#,
        "build_list_count",
    );

    let output = run_reduced_bridge_fn(&bridge);

    assert!(
        output.status.success(),
        "lust-side list bridge mode failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("bridge-fn build_list_count names:List<String> 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-exprs 2 list(String,String)"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-exprs-tail Number Number"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-call 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-list 1"),
        "unexpected stdout:\n{}",
        stdout
    );
}

#[test]
fn parsed_index_function_shape_runs_through_lust_typechecker_bridge_mode() {
    let bridge = reduced_bridge_fn_from_source(
        r#"
fn first_name(names: List<String>) -> Number
let first: String = names[0]
return 1
end
"#,
        "first_name",
    );

    let output = run_reduced_bridge_fn(&bridge);

    assert!(
        output.status.success(),
        "lust-side index bridge mode failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("bridge-fn first_name names:List<String>,first:String 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-exprs 2 index(ident:names[Number])"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-exprs-tail Number Number"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-index 1"),
        "unexpected stdout:\n{}",
        stdout
    );
}

#[test]
fn parsed_member_function_shape_runs_through_lust_typechecker_bridge_mode() {
    let bridge = reduced_bridge_fn_from_source(
        r#"
type User = { name: String, score: Number }

fn read_name(user: User) -> Number
let name: String = user.name
return 1
end
"#,
        "read_name",
    );

    let output = run_reduced_bridge_fn(&bridge);

    assert!(
        output.status.success(),
        "lust-side member bridge mode failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("bridge-fn read_name user:User,name:String 0"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-exprs 2 member(ident:user.name)"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-exprs-tail Number Number"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-member 1"),
        "unexpected stdout:\n{}",
        stdout
    );
}

#[test]
fn parsed_bridge_shape_mismatch_fails_cleanly_at_bridge_boundary() {
    let mut bridge = reduced_bridge_fn_from_source(
        r#"
type User = { name, score }
enum Result = Ok(User) | Err(String)

fn build_user(name: String, score: Number) -> User
let user: User = User { name: name, score: score }
let renamed: String = user.rename("neo")
let wrapped: Result = Ok(user)
let cloned: User = build_user(name, score)
if score > 0 then
    return build_user(name, score)
else
    return user
end
end
"#,
        "build_user",
    );

    bridge.declared_types[1] = "Number".to_string();

    let output = run_reduced_bridge_fn(&bridge);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("runtime error: lust assert failed: unsupported bridged method shape for build_user: expected 1, got 0"),
        "unexpected stdout:\n{}",
        stdout
    );
}

#[test]
fn parsed_real_function_diagnostic_flows_through_lust_bridge() {
    let bridge = reduced_bridge_fn_from_source(
        r#"
type User = { name, score }

fn bad_bridge(name: String, score: Number) -> User
let alias: Number = "oops"
let renamed: String = name.rename("neo")
return name
end
"#,
        "bad_bridge",
    );

    let output = run_reduced_bridge_fn(&bridge);

    assert!(
        output.status.success(),
        "real failing bridge case did not run:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("bridge-fn bad_bridge name:String,score:Number,renamed:String 2"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("initializer for 'alias': expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("type mismatch in return value: expected User, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-method 1"),
        "unexpected stdout:\n{}",
        stdout
    );
}

#[test]
fn parsed_richer_real_function_diagnostic_flows_through_lust_bridge() {
    let bridge = reduced_bridge_fn_from_source(
        r#"
type User = { name, score }
enum Result = Ok(User) | Err(String)

fn bad_combo(name: String, score: Number) -> User
let user: User = User { name: name, score: score }
let renamed: String = user.rename("neo")
let wrapped: Result = Ok(user)
let alias: Number = "oops"
return name
end
"#,
        "bad_combo",
    );

    let output = run_reduced_bridge_fn(&bridge);

    assert!(
        output.status.success(),
        "richer failing bridge case did not run:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("bridge-fn bad_combo name:String,score:Number,user:User,renamed:String,wrapped:Result 2"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("initializer for 'alias': expected Number, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("type mismatch in return value: expected User, got String"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-struct 1"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-method 1"),
        "unexpected stdout:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-enum 1"),
        "unexpected stdout:\n{}",
        stdout
    );
}

#[test]
fn rust_and_lust_bridge_share_core_initializer_mismatch_diagnostic() {
    let src = r#"
type User = { name, score }
enum Result = Ok(User) | Err(String)

fn bad_combo(name: String, score: Number) -> User
let user: User = User { name: name, score: score }
let renamed: String = user.rename("neo")
let wrapped: Result = Ok(user)
let alias: Number = "oops"
return name
end
"#;

    let decls = parse_source(src);
    let rust_errs = TypeChecker::new()
        .check(&decls)
        .expect_err("expected Rust typechecker mismatch");
    let expected_fragment = "initializer for 'alias'";
    assert!(
        rust_errs.iter().any(|e| e.contains(expected_fragment) && e.contains("expected Number") && e.contains("got String")),
        "unexpected Rust typechecker diagnostics: {:?}",
        rust_errs
    );

    let bridge = reduced_bridge_fn_from_source(src, "bad_combo");
    let output = run_reduced_bridge_fn(&bridge);
    assert!(
        output.status.success(),
        "Lust bridge diagnostic run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("initializer for 'alias': expected Number, got String"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
}

#[test]
fn rust_and_lust_bridge_share_list_initializer_mismatch_diagnostic() {
    let src = r#"
fn bad_list_build() -> Number
let names: List<String> = [1, 2]
return 0
end
"#;

    let decls = parse_source(src);
    let rust_errs = TypeChecker::new()
        .check(&decls)
        .expect_err("expected Rust typechecker list mismatch");
    assert!(
        rust_errs.iter().any(|e| e.contains("initializer for 'names'") && e.contains("expected List<String>") && e.contains("got List<Number>")),
        "unexpected Rust typechecker diagnostics: {:?}",
        rust_errs
    );

    let bridge = reduced_bridge_fn_from_source(src, "bad_list_build");
    let output = run_reduced_bridge_fn(&bridge);
    assert!(
        output.status.success(),
        "Lust bridge list diagnostic run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("initializer for 'names': expected List<String>, got List<Number>"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-list 1"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
}

#[test]
fn rust_and_lust_bridge_share_index_initializer_mismatch_diagnostic() {
    let src = r#"
fn bad_first_name(names: List<String>) -> Number
let first: Number = names[0]
return 0
end
"#;

    let decls = parse_source(src);
    let rust_errs = TypeChecker::new()
        .check(&decls)
        .expect_err("expected Rust typechecker index mismatch");
    assert!(
        rust_errs.iter().any(|e| e.contains("initializer for 'first'") && e.contains("expected Number") && e.contains("got String")),
        "unexpected Rust typechecker diagnostics: {:?}",
        rust_errs
    );

    let bridge = reduced_bridge_fn_from_source(src, "bad_first_name");
    let output = run_reduced_bridge_fn(&bridge);
    assert!(
        output.status.success(),
        "Lust bridge index diagnostic run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("initializer for 'first': expected Number, got String"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-index 1"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
}

#[test]
fn rust_and_lust_bridge_share_member_initializer_mismatch_diagnostic() {
    let src = r#"
type User = { name: String, score: Number }

fn bad_read_name(user: User) -> Number
let name: Number = user.name
return 0
end
"#;

    let decls = parse_source(src);
    let rust_errs = TypeChecker::new()
        .check(&decls)
        .expect_err("expected Rust typechecker member mismatch");
    assert!(
        rust_errs.iter().any(|e| e.contains("initializer for 'name'") && e.contains("expected Number") && e.contains("got String")),
        "unexpected Rust typechecker diagnostics: {:?}",
        rust_errs
    );

    let bridge = reduced_bridge_fn_from_source(src, "bad_read_name");
    let output = run_reduced_bridge_fn(&bridge);
    assert!(
        output.status.success(),
        "Lust bridge member diagnostic run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("initializer for 'name': expected Number, got String"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-member 1"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
}

#[test]
fn rust_and_lust_bridge_share_core_return_mismatch_diagnostic() {
    let src = r#"
type User = { name, score }

fn bad_return(name: String, score: Number) -> User
let user: User = User { name: name, score: score }
let renamed: String = user.rename("neo")
return name
end
"#;

    let decls = parse_source(src);
    let rust_errs = TypeChecker::new()
        .check(&decls)
        .expect_err("expected Rust typechecker return mismatch");
    assert!(
        rust_errs
            .iter()
            .any(|e| e.contains("return value") && e.contains("expected User") && e.contains("got String")),
        "unexpected Rust typechecker diagnostics: {:?}",
        rust_errs
    );

    let bridge = reduced_bridge_fn_from_source(src, "bad_return");
    let output = run_reduced_bridge_fn(&bridge);
    assert!(
        output.status.success(),
        "Lust bridge return diagnostic run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("type mismatch in return value: expected User, got String"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-struct 1"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-method 1"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
}

#[test]
fn rust_and_lust_bridge_share_core_call_argument_mismatch_diagnostic() {
    let src = r#"
fn add(a: Number, b: Number) -> Number
return a + b
end

fn bad_call() -> Number
return add("oops", 1)
end
"#;

    let decls = parse_source(src);
    let rust_errs = TypeChecker::new()
        .check(&decls)
        .expect_err("expected Rust typechecker call argument mismatch");
    assert!(
        rust_errs.iter().any(|e| e.contains("argument 1 in call to 'add'") && e.contains("expected Number") && e.contains("got String")),
        "unexpected Rust typechecker diagnostics: {:?}",
        rust_errs
    );

    let bridge = reduced_bridge_fn_from_source(src, "bad_call");
    let output = run_reduced_bridge_fn(&bridge);
    assert!(
        output.status.success(),
        "Lust bridge call diagnostic run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("argument 1 in call to 'add': expected Number, got String"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-call 1"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
}

#[test]
fn rust_and_lust_bridge_share_core_call_arity_mismatch_diagnostic() {
    let src = r#"
fn add(a: Number, b: Number) -> Number
return a + b
end

fn bad_call() -> Number
return add(1)
end
"#;

    let decls = parse_source(src);
    let rust_errs = TypeChecker::new()
        .check(&decls)
        .expect_err("expected Rust typechecker call arity mismatch");
    assert!(
        rust_errs
            .iter()
            .any(|e| e.contains("arity mismatch in call to 'add'") && e.contains("expected 2") && e.contains("got 1")),
        "unexpected Rust typechecker diagnostics: {:?}",
        rust_errs
    );

    let bridge = reduced_bridge_fn_from_source(src, "bad_call");
    let output = run_reduced_bridge_fn(&bridge);
    assert!(
        output.status.success(),
        "Lust bridge call arity diagnostic run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("arity mismatch in call to 'add': expected 2, got 1"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
    assert!(
        normalized.contains("bridge-call 1"),
        "unexpected Lust bridge diagnostics:\n{}",
        stdout
    );
}

#[test]
fn parser_accepts_generic_list_type_annotations_in_let_bindings() {
    let decls = parse_source(
        r#"
fn build_list_count() -> Number
let names: List<String> = ["ana", "bob"]
return 2
end
"#,
    );
    let bridge = reduced_bridge_fn_from_source(
        r#"
fn build_list_count() -> Number
let names: List<String> = ["ana", "bob"]
return 2
end
"#,
        "build_list_count",
    );

    assert!(
        matches!(decls.first(), Some(crate::ast::Decl::Fn(_, _, _, _, body)) if matches!(body.first(), Some(crate::ast::Stmt::Let(_, name, Some(declared), crate::ast::Expr::List(items))) if name == "names" && declared == "List<String>" && items.len() == 2)),
        "expected parsed generic list let binding, got {decls:?}"
    );
    assert_eq!(bridge.declared_types.first().map(String::as_str), Some("List<String>"));
    assert_eq!(
        bridge.expr_shapes.first().map(String::as_str),
        Some("list(String,String)")
    );
}
