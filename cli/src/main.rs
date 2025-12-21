use clap::Parser as ClapParser;
use std::{env, fs::File, io::BufReader};
//use duka_backend::{codegen::Generator, vm::ExeState};
use duka_frontend::{
    analyzer::{Adapter, Analyzer},
    lexer::Lexer,
    parser::Parser,
};
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaLexer, DukaParser};

#[derive(ClapParser, Debug)]
#[command(version, about)]
struct Args {
    #[arg(short, long)]
    name: String,
}

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
    let mut chunk = match Parser::new(lex).parse() {
        Ok(k) => k,
        Err(e) => {
            eprint!("{}", e);
            return;
        }
    };

    let errs = Analyzer.analyze(&chunk);
    if !errs.is_empty() {
        eprintln!("{:?}", errs);
        return;
    }
    Adapter.adapt(&mut chunk);

    // let res = Generator::new().generate(chunk);
    // ExeState::new().execute(&res);
}
