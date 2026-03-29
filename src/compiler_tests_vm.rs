use super::*;

#[test]
fn vm_executes_basic_arithmetic_and_assignment() {
    let vm = run_vm_snippet(
        r#"
let x = 2
let y = x + 3
y = y * 4
print(y)
"#,
    );

    assert_eq!(vm.output(), &["20".to_string()]);
}

#[test]
fn vm_executes_if_and_while() {
    let vm = run_vm_snippet(
        r#"
let i = 0
let sum = 0
while i < 5 do
    sum = sum + i
    i += 1
end

if sum == 10 then
    print("ok", sum)
else
    print("bad")
end
"#,
    );

    assert_eq!(vm.output(), &["ok 10".to_string()]);
}

#[test]
fn vm_executes_string_concat_via_shared_expr_ir() {
    let vm = run_vm_snippet(
        r#"
let greeting = "lust" + " vm"
print(greeting)
"#,
    );

    assert_eq!(vm.output(), &["lust vm".to_string()]);
}

#[test]
fn vm_executes_raw_strings_without_escape_noise() {
    let vm = run_vm_snippet(
        r##"
let pattern = r#"start then "BR/ORD#|" then digits then end"#
print(pattern)
print(lustgex_match("BR/ORD#|78421", pattern))
print(r#"${name} stays literal"#)
"##,
    );

    assert_eq!(
        vm.output(),
        &[
            r#"start then "BR/ORD#|" then digits then end"#.to_string(),
            "true".to_string(),
            "${name} stays literal".to_string(),
        ]
    );
}

#[test]
fn vm_executes_break_and_continue() {
    let vm = run_vm_snippet(
        r#"
let i = 0
let sum = 0
while i < 8 do
    i += 1
    if i % 2 == 0 then
        continue
    end
    if i > 5 then
        break
    end
    sum = sum + i
end
print("loop", sum)
"#,
    );

    assert_eq!(vm.output(), &["loop 9".to_string()]);
}

#[test]
fn vm_executes_for_loops_via_shared_stmt_ir() {
    let vm = run_vm_snippet(
        r#"
let sum = 0
let text = ""

for n in [1, 2, 3, 4, 5] do
    if n == 2 then
        continue
    end
    if n == 5 then
        break
    end
    sum = sum + n
end

for ch in "axe" do
    text = text + ch
end

print(sum, text)
"#,
    );

    assert_eq!(vm.output(), &["8 axe".to_string()]);
}

#[test]
fn vm_executes_numeric_range_for_loops() {
    let vm = run_vm_snippet(
        r#"
let sum = 0
for i in 0..5 do
    sum += i
end
print("sum", sum)
"#,
    );

    assert_eq!(vm.output(), &["sum 10".to_string()]);
}

#[test]
fn vm_executes_inclusive_numeric_range_for_loops() {
    let vm = run_vm_snippet(
        r#"
let sum = 0
for i in 1..=5 do
    sum += i
end
print("sum", sum)
"#,
    );

    assert_eq!(vm.output(), &["sum 15".to_string()]);
}

#[test]
fn vm_executes_indexed_iteration_for_loops() {
    let vm = run_vm_snippet(
        r#"
let total = 0
let labels = ""

for i, n in [3, 4, 5] do
    total += i + n
    labels = labels + to_string(i) + ":" + to_string(n) + ";"
end

print("total", total)
print("labels", labels)
"#,
    );

    assert_eq!(vm.output(), &["total 15".to_string(), "labels 0:3;1:4;2:5;".to_string()]);
}

#[test]
fn vm_executes_dict_indexing_and_assignment() {
    let vm = run_vm_snippet(
        r#"
let scores = dict("ada", 3, "grace", 5)
scores["ada"] += 2
scores["alan"] = 7
print(scores["ada"], scores["alan"], scores["missing"] == null, type_of(scores))
"#,
    );

    assert_eq!(vm.output(), &["5 7 true Map".to_string()]);
}

#[test]
fn vm_executes_map_literals_and_methods() {
    let vm = run_vm_snippet(
        r#"
let scores = { "ada": 3, "grace": 5 }
scores.set("alan", 7)
print(scores.length(), scores.has("ada"), scores.has("linus"))
print(scores.keys().length(), scores.values().length(), scores["alan"])
"#,
    );

    assert_eq!(
        vm.output(),
        &["3 true false".to_string(), "3 3 7".to_string()]
    );
}

#[test]
fn vm_executes_json_parse_into_maps_and_lists() {
    let vm = run_vm_snippet(
        r#"
let data = json_parse("{\"name\":\"Ada\",\"scores\":[3,5],\"meta\":{\"active\":true}}")
print(data["name"], data["scores"][1], data["meta"]["active"], type_of(data))
"#,
    );

    assert_eq!(vm.output(), &["Ada 5 true Map".to_string()]);
}

#[test]
fn vm_matches_read_file_result_ok_and_err_variants() {
    let vm = run_vm_snippet(
        r#"
match read_file_result("Cargo.toml") do
    case FileOk(content) then
        print("ok", content.length() > 0)
    case FileErr(message) then
        print("unexpected", message)
end

match read_file_result("definitely_missing_lust_test_file.txt") do
    case FileOk(_) then
        print("unexpected_ok")
    case FileErr(_) then
        print("missing")
end
"#,
    );

    assert_eq!(vm.output(), &["ok true".to_string(), "missing".to_string()]);
}

#[test]
fn vm_executes_map_entries_and_json_roundtrip() {
    let vm = run_vm_snippet(
        r#"
let scores = { "ada": 3, "grace": 5 }
let text = ""

for entry in scores.entries() do
    let [key, value] = entry
    text = text + key + ":" + to_string(value) + ";"
end

let encoded = json_encode(scores)
let decoded = json_parse(encoded)

print(text)
print(encoded.contains("\"ada\":3"), decoded["grace"])
"#,
    );

    assert_eq!(
        vm.output(),
        &["ada:3;grace:5;".to_string(), "true 5".to_string()]
    );
}

#[test]
fn vm_executes_direct_key_value_map_iteration() {
    let vm = run_vm_snippet(
        r#"
let scores = { "ada": 3, "grace": 5 }
let text = ""

for key, value in scores do
    text = text + key + ":" + to_string(value) + ";"
end

print(text)
"#,
    );

    assert_eq!(vm.output(), &["ada:3;grace:5;".to_string()]);
}

#[test]
fn vm_executes_map_value_transforms() {
    let vm = run_vm_snippet(
        r#"
let scores = { "ada": 3, "grace": 5, "linus": 8 }
let doubled = map_values(scores, fn(v) => v * 2)
let filtered = filter_values(doubled, fn(v) => v > 8)

print(doubled["ada"], doubled["grace"], doubled["linus"])
print(filtered.has("ada"), filtered.has("grace"), filtered.has("linus"))
"#,
    );

    assert_eq!(
        vm.output(),
        &["6 10 16".to_string(), "false true true".to_string()]
    );
}

#[test]
fn vm_executes_map_entry_transforms() {
    let vm = run_vm_snippet(
        r#"
let scores = { "ada": 3, "grace": 5, "linus": 8 }
let renamed = map_entries(scores, fn(entry) => [entry[0] + "_x", entry[1] + 1])
let filtered = filter_entries(renamed, fn(entry) => entry[1] > 6)

print(renamed["ada_x"], renamed["grace_x"], renamed["linus_x"])
print(filtered.has("ada_x"), filtered.has("grace_x"), filtered.has("linus_x"))
"#,
    );

    assert_eq!(
        vm.output(),
        &["4 6 9".to_string(), "false false true".to_string()]
    );
}

#[test]
fn vm_executes_method_style_map_transforms() {
    let vm = run_vm_snippet(
        r#"
let scores = { "ada": 3, "grace": 5, "linus": 8 }
let doubled = scores.map_values(fn(v) => v * 2)
let filtered = doubled.filter_values(fn(v) => v > 8)
let renamed = filtered.map_entries(fn(entry) => [entry[0] + "_done", entry[1]])

print(doubled["ada"], doubled["grace"], doubled["linus"])
print(filtered.has("ada"), filtered.has("grace"), filtered.has("linus"))
print(renamed.has("grace_done"), renamed["linus_done"])
"#,
    );

    assert_eq!(
        vm.output(),
        &[
            "6 10 16".to_string(),
            "false true true".to_string(),
            "true 16".to_string(),
        ]
    );
}

#[test]
fn vm_executes_for_pattern_destructuring() {
    let vm = run_vm_snippet(
        r#"
let text = ""
for [key, value] in { "ada": 3, "grace": 5 }.entries() do
    text = text + key + ":" + to_string(value) + ";"
end
print(text)
"#,
    );

    assert_eq!(vm.output(), &["ada:3;grace:5;".to_string()]);
}

#[test]
fn vm_executes_function_calls_and_returns() {
    let vm = run_vm_snippet(
        r#"
fn add(a, b)
    let sum = a + b
    return sum
end

fn mul_add(x, y, z)
    return add(x * y, z)
end

let result = mul_add(2, 3, 4)
print(result)
"#,
    );

    assert_eq!(vm.output(), &["10".to_string()]);
}

#[test]
fn vm_keeps_operand_stack_balanced_across_function_calls() {
    let vm = run_vm_snippet(
        r#"
fn bump(n)
    let label = "n=" + to_string(n)
    return n + 1
end

let xs = []
let i = 0
while i < 3 do
    xs.push(bump(i))
    i += 1
end

print(xs.length(), xs[0], xs[2])
"#,
    );

    assert_eq!(vm.output(), &["3 1 3".to_string()]);
}

#[test]
fn vm_executes_lists_indexing_and_list_methods() {
    let vm = run_vm_snippet(
        r#"
let xs = [1, 2]
xs.push(3)
xs[1] = 9
print(xs[0], xs[1], xs[2], xs.length())
"#,
    );

    assert_eq!(vm.output(), &["1 9 3 3".to_string()]);
}

#[test]
fn vm_executes_struct_fields_and_assignment() {
    let vm = run_vm_snippet(
        r#"
type Pair = { left, right }

fn total(pair)
    return pair.left + pair.right
end

let pair = Pair { left: 2, right: 3 }
pair.right = 9
print(pair.left, pair.right, total(pair))
"#,
    );

    assert_eq!(vm.output(), &["2 9 11".to_string()]);
}

#[test]
fn vm_executes_struct_methods_with_self() {
    let vm = run_vm_snippet(
        r#"
type Counter = { value }

fn Counter.inc()
    self.value = self.value + 1
end

fn Counter.next()
    return self.value + 1
end

let c = Counter { value: 4 }
c.inc()
print(c.value, c.next())
"#,
    );

    assert_eq!(vm.output(), &["5 6".to_string()]);
}

#[test]
fn vm_executes_enum_values_and_constructors() {
    let vm = run_vm_snippet(
        r#"
enum Option = Some(value) | None

fn show(opt)
    match opt do
        case Some(v) then
            print("some", v)
        case None then
            print("none")
    end
end

show(Some(7))
show(None)
"#,
    );

    assert_eq!(vm.output(), &["some 7".to_string(), "none".to_string()]);
}

#[test]
fn vm_executes_match_with_enum_guard_and_fallback() {
    let vm = run_vm_snippet(
        r#"
enum Msg = Note(note) | Quit

let msg = Note("C-4")

match msg do
    case Note(n) if n == "C-4" then
        print("hit", n)
    case other then
        print("miss", other)
end
"#,
    );

    assert_eq!(vm.output(), &["hit C-4".to_string()]);
}

#[test]
fn vm_executes_struct_pattern_with_binding_and_guard() {
    let vm = run_vm_snippet(
        r#"
type User = { name, age }

let user = User { name: "Kid", age: 17 }

match user do
    case User { name: "Admin" } then
        print("boss")
    case User { age: a } if a < 18 then
        print("young", a)
end
"#,
    );

    assert_eq!(vm.output(), &["young 17".to_string()]);
}

#[test]
fn vm_executes_nested_struct_pattern() {
    let vm = run_vm_snippet(
        r#"
type Profile = { age }
type User = { name, profile }

let user = User { name: "Ana", profile: Profile { age: 20 } }

match user do
    case User { profile: Profile { age: age } } then
        print("profile", age)
end
"#,
    );

    assert_eq!(vm.output(), &["profile 20".to_string()]);
}

#[test]
fn vm_executes_list_patterns() {
    let vm = run_vm_snippet(
        r#"
let xs = [4, 7]

match xs do
    case [4, value] then
        print("pair", value)
    case _ then
        print("miss")
end
"#,
    );

    assert_eq!(vm.output(), &["pair 7".to_string()]);
}

#[test]
fn vm_executes_list_rest_patterns() {
    let vm = run_vm_snippet(
        r#"
let xs = [7, 9, 11]

match xs do
    case [first, ..] then
        print("rest", first)
    case _ then
        print("miss")
end
"#,
    );

    assert_eq!(vm.output(), &["rest 7".to_string()]);
}

#[test]
fn vm_executes_destructuring_let_pattern() {
    let vm = run_vm_snippet(
        r#"
let [name, id, ..] = " Jacob ,101,extra " |> split(",")
print(name.trim(), id.trim())
"#,
    );

    assert_eq!(vm.output(), &["Jacob 101".to_string()]);
}

#[test]
fn vm_executes_nested_enum_struct_patterns() {
    let vm = run_vm_snippet(
        r#"
type User = { name }
enum Message = Joined(user) | Quit

let msg = Joined(User { name: "Ana" })

match msg do
    case Joined(User { name: name }) then
        print(name)
    case Quit then
        print("quit")
end
"#,
    );

    assert_eq!(vm.output(), &["Ana".to_string()]);
}

#[test]
fn vm_executes_builtin_file_io_and_to_string() {
    let temp_path = std::env::temp_dir().join("lust_vm_builtin_test.txt");
    let temp_path_str = temp_path.to_string_lossy().to_string();
    let src = format!(
        r#"
write_file("{}", "hello")
let content = read_file("{}")
print(to_string(42), content)
"#,
        temp_path_str, temp_path_str
    );

    let vm = run_vm_snippet(&src);
    assert_eq!(vm.output(), &["42 hello".to_string()]);
}

#[test]
fn vm_executes_file_handle_methods() {
    let dir = fresh_temp_dir("lust_vm_file_handle_io");
    let path = dir.join("stream.txt");
    let src = format!(
        r#"
let file = open_file("{}", "write")
file.write("alpha")
file.write_line("-beta")
file.write_line("gamma")
file.close()
print("done")
"#,
        path.display()
    );

    let vm = run_vm_snippet(&src);
    assert_eq!(vm.output(), &["done".to_string()]);

    let written = fs::read_to_string(&path).expect("expected vm file-handle output");
    assert_eq!(written, "alpha-beta\ngamma\n");
}

#[test]
fn vm_executes_json_builtins() {
    let vm = run_vm_snippet(
        r#"
let encoded = json_encode("hi")
let decoded = json_decode("[1,2,3]")
print(encoded)
print(decoded)
"#,
    );

    assert_eq!(vm.output(), &["\"hi\"".to_string(), "[\n  1,\n  2,\n  3\n]".to_string()]);
}

#[test]
fn vm_executes_get_key_builtin() {
    let vm = run_vm_snippet_with_args_and_keys(
        r#"
let key = get_key()
match key do
    case Up then
        print("up")
    case Char(ch) then
        print("char", ch)
    case _ then
        print("other")
end
"#,
        vec![],
        vec!["up\n".to_string()],
    );

    assert_eq!(vm.output(), &["up".to_string()]);
}

#[test]
fn vm_executes_input_builtin() {
    let vm = run_vm_snippet_with_runtime_inputs(
        r#"
let name = input()
print("hello", name)
"#,
        vec![],
        vec![],
        vec!["capsule\n".to_string()],
    );

    assert_eq!(vm.output(), &["hello capsule".to_string()]);
}

#[test]
fn vm_executes_debug_and_assert_builtins() {
    let vm = run_vm_snippet(
        r#"
debug("vm", 7)
assert(1 == 1, "should pass")
print("ok")
"#,
    );

    assert_eq!(vm.output(), &["DEBUG vm 7".to_string(), "ok".to_string()]);
}

#[test]
fn vm_executes_type_of_builtin() {
    let vm = run_vm_snippet(
        r#"
type User = { name }
enum Message = Joined(user)

let user = User { name: "Ana" }
let msg = Joined(user)
print(type_of(7), type_of("hi"), type_of(null), type_of(user), type_of(msg))
"#,
    );

    assert_eq!(
        vm.output(),
        &["Number String Null Struct:User Enum:Message.Joined".to_string()]
    );
}
