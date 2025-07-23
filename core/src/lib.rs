pub mod backend;
pub mod frontend;
pub mod shared;

pub use backend::codegen::generate;
pub use backend::vm::ExeState;
pub use frontend::lexer::Lexer;
pub use frontend::parser::Parser;

#[cfg(test)]
mod tests {

    use crate::{
        Parser,
        backend::vm::{ExeState, instructions::Instruction},
        frontend::{lexer::Lexer, token::TokenKind},
        generate,
        shared::types::{DukaLexer, DukaVM},
    };
    use std::{io::Cursor, mem};

    macro_rules! from_string {
        ($s: expr) => {
            Lexer::new(Cursor::new($s))
        };
    }
    macro_rules! print_tokens {
        ($lex: ident) => {
            loop {
                match $lex.next() {
                    Ok(t) if t.0.is_terminator() => break,
                    Ok(t) => println!("{:?}", t),
                    Err(e) => panic!("{:?}", e),
                }
            }
        };
    }
    macro_rules! expect_kinds {
        ($lex: ident match) => {
            match $lex.next() {
                Ok(t) => {
                    println!("end");
                    assert!(t.0 == TokenKind::EOF);
                }
                Err(e) => panic!("{:?}", e),
            }
        };

        ($lex: ident match $cur: expr $(, $rest: expr)* $(,)?) => {
            match $lex.next() {
                Ok(t) => {
                    println!("{:#?}", t);
                    assert!(t.0 == $cur);
                    expect_kinds!($lex match $($rest),*);
                }
                Err(e) => panic!("{:?}", e),
            }
        };
    }

    #[test]
    fn macro_inst_test() {
        let i = Instruction::GetGlobal(1, 2);
        let r = Instruction::Return0();
        let s = Instruction::LoadConst(0, 2);
        println!("{}", mem::size_of::<Instruction>());
        println!("{}", i);
        println!("{}", 0f32 == 0f32);
        println!("{}", (-0f32) == 0f32);
        println!("{:?}", i.mode());
        println!("{:?}", i.check_setA());
        println!("{:b}", r.raw());
        println!("{:?}", r.decode());
        println!("{:b}", s.raw());
        println!("{:?}", s.decode());
    }

    #[test]

    fn new_parser_test() {
        println!(
            "{:#?}",
            Parser::new(from_string!(
                r#"
            local a <const> = (1+1) <<not ~ - print(1)
            "#
            ))
            .parse()
            .unwrap()
        );
    }

    #[test]
    fn parse_proto_test() {
        ExeState::new().execute(&generate(
            Parser::new(from_string!(
                r#"#!user/duka/bin
        
        print [[你好你好]] -- short
        print "fuck off fuck off" -- mid
        -- long
        print "fuck off fuck off fuck off fuck off fuck off fuck off"
        
        
        
        "#
            ))
            .parse()
            .unwrap(),
        ));
    }

    #[test]
    fn lexer_test() {
        let mut l = from_string!(
            r#"{ 1,2,'three',{key=value}}       
        a and b or not c         
        ::label::
        <attr>      
        print('长字符串测试'..tostring(42))"#
        );
        print_tokens!(l);
    }

    #[test]
    fn number_test() {
        let mut l = from_string!(
            r#"
            1
            114514
            -13.334e2
            0b101_010_110_
            0e10
            -0xFFF
            0b101
            0o777
            -13.13.
            2f
            3.3f
            0f
        "#
        );
        print_tokens!(l);
    }

    #[test]
    fn error_test() {
        eprintln!(
            "{}",
            from_string!(
                r#"
"6767676767676\6"#
            )
            .next()
            .err()
            .unwrap()
        );
    }

    #[test]
    fn string_test() {
        let mut l = from_string!(
            r#"
            "\t"
            "\"\\"
            [[
            \s
            ]]
        "#
        );
        expect_kinds! { l match
            TokenKind::String("\t".into()),
            TokenKind::String("\"\\".into()),
            TokenKind::String("\n            \\s\n            ".into()),
        }
    }

    #[test]
    fn utf8_test() {
        let mut l = from_string!(
            r#"
            "你好Б б少し難しかったです😂"
            "\u{25}" -- %
            "\u{3999}\u{5000}" --㦙倀

        "#
        );
        expect_kinds! { l match
            TokenKind::String("你好Б б少し難しかったです😂".into()),
            TokenKind::String("%".into()),
            TokenKind::String("㦙倀".into()),
        }
    }

    #[test]
    fn ml_test() {
        let mut l = from_string!(
            r#"
            --[ss
            --[===[ 
            
            Comment ]==
            
            ]==]===]
[===[
[[String]==] ]==]===]
        "#
        );

        expect_kinds! { l match
            TokenKind::String("[[String]==] ]==".into())
        }
    }
}
