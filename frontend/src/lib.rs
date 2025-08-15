pub mod analyzer;
pub mod lexer;
pub mod parser;

/// # 要用ASSERT!
#[cfg(test)]
mod tests {

    use crate::{analyzer::visitors::*, analyzer::*, lexer::*, parser::*};
    use duka_shared::{error::*, token::*, types::*};
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
                    assert!(t.0.is_terminator());
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

    struct Printer;
    impl Visitor for Printer {
        fn visit_expr(&mut self, _expr: &duka_shared::ast::Expr) {
            println!("{:#?}", _expr);
        }
        fn visit_stmt(&mut self, _stmt: &duka_shared::ast::Stmt) {
            println!("{:#?}", _stmt);
        }
    }

    #[test]
    fn transformer_test() {
        let mut chunk = Parser::new(from_string!(
            r#"
global a = linq!(
    from x in array
    where x > 2
    from y in array2
    select x * y
)
        "#
        ))
        .parse()
        .unwrap();

        transform(&mut ConstFoldTransformer::new(), &mut chunk);
        transform(&mut MeaninglessTransformer::new(), &mut chunk);
        transform(&mut DesugarTransformer::new(), &mut chunk);

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

        let mut er: Vec<DukaError> = vec![];

        er.extend(check(&mut LabelChecker::new(), &chunk));

        er.extend(check(&mut VarArgChecker::new(), &chunk));

        er.extend(check(&mut LoopChecker::new(), &chunk));

        let er: Vec<DukaSemanticError> = er
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
                DukaSemanticError::InvisibleGotoLabel("b".to_owned()),
                DukaSemanticError::InvalidLoopFlowControl,
                DukaSemanticError::InvalidVarArg
            ]
        )
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
        // ExeState::new().execute(&generate(
        //     Parser::new(from_string!(
        //         r#"#!user/duka/bin

        // print [[你好你好]] -- short
        // print "fuck off fuck off" -- mid
        // -- long
        // print "fuck off fuck off fuck off fuck off fuck off fuck off"

        // "#
        //     ))
        //     .parse_chunk()
        //     .unwrap(),
        // ));
    }

    #[test]
    fn lexer_test() {
        let mut l = from_string!(r#"global a"#);
        expect_kinds! { l match
            TokenKind::Global,
            TokenKind::Ident("a".to_owned())
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
    fn macro_test() {
        let mut lex = from_string!(
            r#"
        ^#define tuple(a, ...)
            $a, 
            [:when!(
                [:nonempty!($...(,)):], 
                [:~when!(
                false, 
                [:~~tuple($...(,)):], 
                end
            ):], 
                end
            ):]
        ^#enifed

        --^#define A1(...) -> {$...[;)};

        --[:A1(1,2,3):]
        [:tuple(false, 1, 2, 3):]
        "#
        );
        print_tokens!(lex);
    }
}
