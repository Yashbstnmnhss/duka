pub mod backend;
pub mod frontend;
pub mod shared;

pub use backend::codegen::generate;
pub use backend::vm::ExeState;
pub use frontend::lexer::Lexer;
pub use frontend::parser::Parser;

/// # TODO: 不要单纯用println做测试了
/// # 要用ASSERT!
#[cfg(test)]
mod tests {

    use crate::{
        Parser,
        backend::vm::{
            ExeState,
            instructions::{DecodeInstruction, Instruction, InstructionName},
        },
        frontend::{
            analyzer::Walker,
            lexer::LexerWithMacro,
            token::TokenKind,
            visitors::{
                ConstFoldTransformer, LabelChecker, LoopChecker, MeaninglessTransformer,
                VarArgChecker,
            },
        },
        generate,
        shared::{
            error::{DukaErrorKind, DukaSemanticError},
            types::{DukaLexer, DukaParser, DukaVM},
        },
    };
    use std::io::Cursor;

    macro_rules! from_string {
        ($s: expr) => {
            LexerWithMacro::new(Cursor::new($s))
        };
    }
    macro_rules! print_tokens {
        ($lex: ident) => {
            loop {
                match $lex.next() {
                    Ok(t) => {
                        println!("{:?}", t);
                        if t.0.is_terminator() {
                            break;
                        }
                    }
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
    fn transformer_test() {
        let mut chunk = Parser::new(from_string!(
            r#"
--[[
match target
| a -> ...
| b -> ...
| c -> ...
| else ...
end
]]
(0 |> f(7) <| 2)
if true then
    a = 1+1 |> print
    a,b,c=1,2,3
    function<attr> abc(abc, bc, bc) end
    global a <c,c,b> = 1
end
        "#
        ))
        .parse()
        .unwrap();

        Walker::new()
            .add_transformer(ConstFoldTransformer::new())
            .add_transformer(MeaninglessTransformer::new())
            .transform(&mut chunk);
        println!("{:#?}", chunk)
    }

    #[test]
    fn checker_test() {
        let chunk = Parser::new(from_string!(
            r#"
a = ...    
::b::  
function a()
    a = ...
    function b(...)
            b = ... +1
    end
goto b
end

break
        "#
        ))
        .parse()
        .unwrap();

        let er: Vec<DukaSemanticError> = Walker::new()
            .add_checker(LabelChecker::new())
            .add_checker(LoopChecker::new())
            .add_checker(VarArgChecker::new())
            .check(&chunk)
            .err()
            .unwrap()
            .into_iter()
            .map(|e| {
                if let DukaErrorKind::Semantic(s) = e.kind {
                    s
                } else {
                    unreachable!()
                }
            })
            .collect();

        assert_eq!(
            er,
            vec![
                DukaSemanticError::InvisibleGotoLabel("b".to_string()),
                DukaSemanticError::InvalidLoopFlowControl,
                DukaSemanticError::InvalidVarArg
            ]
        )
    }

    #[test]
    fn instruction_macro_test() {
        let i = Instruction::Move(1, 2);
        assert_eq!(i.decode(), DecodeInstruction::Move(1, 2));
        assert_eq!(i.name(), InstructionName::Move);
        assert_eq!(i.check_setA(), true);
        assert_eq!(Instruction::validate(i.raw()), true);
        let i = Instruction::LoadI(1, -2);
        assert_eq!(i.decode(), DecodeInstruction::LoadI(1, -2));
    }

    #[test]
    fn parser_test() {
        println!(
            "{:#?}",
            Parser::new(from_string!(
                r#"
^^define PI -> 3.1415926
print([:PI:])
        "#
            ))
            .parse_chunk()
            .unwrap()
        )
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
            .parse_chunk()
            .unwrap(),
        ));
    }

    #[test]
    fn lexer_test() {
        let mut l = from_string!(r#"global a"#);
        expect_kinds! { l match
            TokenKind::Global,
            TokenKind::Ident("a".to_string())
        }
    }

    #[test]
    fn number_test() {
        let mut l = from_string!(
            r#"
            1 114514
            0b101_010_110_
            0e10
            0o777
            2f
            3.3f
            0f
        "#
        );
        expect_kinds! { l match
            TokenKind::Int(1),
            TokenKind::Int(114514),
            TokenKind::Int(0b101_010_110_),
            TokenKind::Float(0.0),
            TokenKind::Int(0o777),
            TokenKind::Float(2.0),
            TokenKind::Float(3.3),
            TokenKind::Float(0.0),
        }
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
            TokenKind::String("            \\s\n            ".into()),
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

    #[test]
    fn just_print() {
        let mut lex = from_string!(
            r#"
        ^^define A(b, ...)
            a = [:nameof!(b):]
        ^^enifed
        
        [:A(123):]

        "#
        );
        print_tokens!(lex);
    }
}
