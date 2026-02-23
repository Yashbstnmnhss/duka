//! Frontend of Duka
//!
//! Including lexer, parser, analyzer, adapter

use duka_macros::史書云;
use duka_shared::utils::SemVer;

pub mod analyzer;
pub mod ir;
pub mod lexer;
pub mod parser;

pub mod prelude {
    pub use crate::{
        analyzer::{Adapter, BasicAnalyzer},
        lexer::LexerWithMacro,
        parser::Parser,
    };
}

pub const VERSION: SemVer = 史書云! {
    <<前端>> 者
    為 世家 "項目之創立" 也
    // 失
    為 世家 "Value之優化" 也
};

/// # 要用ASSERT!
#[cfg(test)]
mod tests {

    use crate::{analyzer::visitors::*, analyzer::*, lexer::token::*, lexer::*, parser::*};
    use duka_shared::{errors::*, types::*};
    use std::io::Cursor;

    macro_rules! expect_kinds {
        ($lex: ident match) => {
            match $lex.next() {
                Some(t) => {
                    println!("end");
                    assert!(t.0.is_terminator());
                }
                None => panic!("NO"),
            }
        };

        ($lex: ident match $cur: expr $(, $rest: expr)* $(,)?) => {
            match $lex.next() {
                Some(t) => {
                    println!("{:#?}", t);
                    assert!(t.0 == $cur);
                    expect_kinds!($lex match $($rest),*);
                }
                _ => panic!("NO"),
            }
        };
    }
    macro_rules! from_string {
        ($s: expr) => {
            LexerWithMacro::new(Cursor::new($s), Some("test".into()))
                .tokenize()
                .unwrap()
        };
    }

    #[test]
    fn incomplete_lexer_test() {
        let mut lexer = Lexer::new(
            Cursor::new(
                r#"[[s
        "#,
            ),
            None,
        );
        while let Ok(tk) = lexer.next_kind() {
            println!("{:?}", tk);

            if let DukaResumable::Complete(ref tk) = tk
                && tk.is_terminator()
            {
                break;
            }
            if let DukaResumable::Incomplete(..) = tk {
                break;
            }
        }
    }

    #[test]
    fn transformer_test() {
        let mut chunk = Parser::parse(
            from_string!(
                r#"
global a = linq!(
    from x in array
    where x > 2
    from y in array2
    select x * y
)
global b = match a then
            true -> false;
            {1,...,2} -> true;
            else return false end
        "#
            ),
            Default::default(),
        )
        .unwrap();

        transform(&mut ConstFoldTransformer::new(), &mut chunk);
        transform(&mut MeaninglessTransformer::new(), &mut chunk);
        transform(&mut DesugarTransformer::new(), &mut chunk);

        println!("{:#?}", chunk)
    }

    #[test]
    fn checker_test() {
        let chunk = Parser::parse(
            from_string!(
                r#"
a = ...    
goto b
function a()
    a = ...
    function b(...)
            b = ... +1
    end
goto b
end
::b::  
break
        "#
            ),
            Default::default(),
        )
        .unwrap();

        let mut er: Vec<DukaSpannedError> = vec![];

        let (data, _) = ScopeAnalyzer.analyze(&chunk, Default::default());
        er.extend(check(
            &mut LabelChecker::new(chunk.source_info.clone(), &data),
            &chunk,
        ));

        er.extend(check(
            &mut VarArgChecker::new(chunk.source_info.clone(), &data),
            &chunk,
        ));

        er.extend(check(
            &mut LoopChecker::new(chunk.source_info.clone(), &data),
            &chunk,
        ));

        let er: Vec<DukaSemanticError> = er
            .into_iter()
            .map(|e| {
                if let DukaErrorKind::Semantic(err) = e.kind {
                    err
                } else {
                    unreachable!()
                }
            })
            .collect();

        assert_eq!(
            er,
            vec![
                DukaSemanticError::InvisibleGotoLabel("b".into()),
                DukaSemanticError::InvalidVarArg,
                DukaSemanticError::InvalidLoopFlowControl,
            ]
        )
    }

    #[test]
    fn parse_logic_test() {
        let _ = Parser::parse(
            from_string!(
                r#"
logic! {
    rule test() =
        if parent(X, Y) then ancestor(X, Y)
}
        "#
            ),
            Default::default(),
        )
        .unwrap();
    }

    #[test]
    fn lexer_test() {
        let mut l = from_string!(r#"global a"#).tokens.into_iter();
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
        )
        .tokens
        .into_iter();
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
        )
        .tokens
        .into_iter();
        expect_kinds! { l match
            TokenKind::String("\t".as_bytes().into()),
            TokenKind::String("\"\\".as_bytes().into()),
            TokenKind::String("            \\s\n            ".as_bytes().into()),
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
        )
        .tokens
        .into_iter();
        expect_kinds! { l match
            TokenKind::String("你好Б б少し難しかったです😂".as_bytes().into()),
            TokenKind::String("%".as_bytes().into()),
            TokenKind::String("㦙倀".as_bytes().into()),
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
        )
        .tokens
        .into_iter();

        expect_kinds! { l match
            TokenKind::String("[[String]==] ]==".as_bytes().into())
        }
    }

    #[test]
    #[should_panic]
    fn macro_recursion_test() {
        let _ = from_string!(
            r#"
            ^#define test()
                [:test():]
            ^#enifed
            [:test():]
        "#
        );
    }

    #[test]
    fn macro_test() {
        let mut lex = from_string!(
            r#"
        ^#define tuple(a, ...)
            $a, 
            [:when!(
                [:nonempty!($...(,)):],
                [:~tuple($...(,)):],
                end
            ):]
        ^#enifed

        --^#define A1(...) -> {$...[;)};

        --[:A1(1,2,3):]
        [:tuple(1, 2, 3):]
        "#
        )
        .tokens
        .into_iter();
        expect_kinds!(lex match
            TokenKind::Int(1),
            TokenKind::Comma,
            TokenKind::Int(2),
            TokenKind::Comma,
            TokenKind::Int(3),
            TokenKind::Comma,
            TokenKind::End
        );
    }
}
