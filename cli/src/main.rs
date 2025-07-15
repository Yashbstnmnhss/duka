use std::{env, fs::File, io::BufReader};

use duka::{
    Parser, backend::vm::ExeState, frontend::lexer::Lexer, generate, shared::types::DukaVM,
};

fn main() {
    println!("Duka Interpreter");
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        eprintln!("Error: Expecting path of script file");
        println!("Usage: {} <script>", args[0]);
        return;
    }
    let script_path = &args[1];
    let input = File::open(script_path).expect("Unable to open file");
    let lex = Lexer::new(BufReader::new(input));
    let res = Parser::new(lex).parse();
    match res {
        Ok(prog) => ExeState::new().execute(&generate(prog)),
        Err(e) => eprintln!("Error: {:?}", e),
    }
}
