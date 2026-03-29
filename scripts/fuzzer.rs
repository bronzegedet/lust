use std::fs;
use rand::seq::SliceRandom;
use rand::Rng;

fn main() {
    let test_dir = "tests/fuzz";
    fs::create_dir_all(test_dir).unwrap();

    let components = vec![
        "fn", "type", "let", "if", "while", "spawn", "end", "do", "then", "else",
        "=", "==", "!=", ">", "<", "+", "-", "*", "/", "[", "]", "(", ")", "{", "}", ".", ",",
        "10", "20", "3.14", "\"hello\"", "x", "y", "z", "my_var", "Robot", "tasks", "print", "factorial"
    ];

    for i in 0..100 {
        let mut fuzzed_code = String::new();
        let length = rand::thread_rng().gen_range(10..50);
        
        for _ in 0..length {
            let part = components.choose(&mut rand::thread_rng()).unwrap();
            fuzzed_code.push_str(part);
            fuzzed_code.push(' ');
        }

        let file_path = format!("{}/garbage_{}.lust", test_dir, i);
        fs::write(file_path, fuzzed_code).unwrap();
    }
    println!("Generated 100 garbage Lust files in {}", test_dir);
}
