use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet, SyntaxSetBuilder};

fn setup() -> (SyntaxSet, SyntaxReference) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut builder = SyntaxSetBuilder::new();
    builder.add_from_folder(dir, true).unwrap();
    let set = builder.build();
    let syn = set.find_syntax_by_extension("duka").unwrap().clone();
    (set, syn)
}

/// 逐行解析,返回 (scope 串, token 文本) 序列
fn toks(code: &str, set: &SyntaxSet, syn: &SyntaxReference) -> Vec<(String, String)> {
    let mut ps = ParseState::new(syn);
    let mut stack = ScopeStack::new();
    let mut out = vec![];
    let mut line_no = 0;
    for line in code.lines() {
        line_no += 1;
        let ops = ps
            .parse_line(line, set)
            .unwrap_or_else(|e| panic!("parse failed at line {line_no}: {e}"));
        let mut start = 0;
        for (idx, op) in ops {
            if idx > start {
                push_region(&mut out, &stack, &line[start..idx]);
            }
            stack
                .apply(&op)
                .unwrap_or_else(|e| panic!("scope apply failed: {e}"));
            start = idx;
        }
        if start < line.len() {
            push_region(&mut out, &stack, &line[start..]);
        }
    }
    out
}

fn push_region(out: &mut Vec<(String, String)>, stack: &ScopeStack, text: &str) {
    let s = stack
        .as_slice()
        .iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    out.push((s, text.to_string()));
}

#[test]
fn grammar_file_is_loadable() {
    // duka.sublime-syntax 必须能被 syntect 解析
    let (set, syn) = setup();
    assert_eq!(syn.name, "duka");
    let _ = set;
}

#[test]
fn pipeline_token_kept_whole() {
    // |> 应作为整体 token,不能被拆成 bitwise | + comparison >
    let (set, syn) = setup();
    let tokens = toks("local a = x |> f()", &set, &syn);
    let pipe = tokens
        .iter()
        .find(|(_, t)| t == "|>")
        .expect("|> token not kept whole");
    assert!(pipe.0.contains("pipeline"), "|> scope: {}", pipe.0);
}

#[test]
fn no_ampamp_logical() {
    // && 不存在:不该作为单个逻辑 token,也不该有 logical scope
    let (set, syn) = setup();
    let tokens = toks("local a = x && y", &set, &syn);
    assert!(!tokens.iter().any(|(_, t)| t == "&&"), "&& treated as one token");
    assert!(
        !tokens.iter().any(|(s, _)| s.contains("logical")),
        "found bogus logical operator"
    );
}

#[test]
fn unicode_identifier() {
    let (set, syn) = setup();
    let tokens = toks("local 変数 = 1", &set, &syn);
    let id = tokens
        .iter()
        .find(|(_, t)| t == "変数")
        .expect("unicode identifier not tokenized");
    assert!(id.0.contains("variable.other"), "scope: {}", id.0);
}

#[test]
fn sugar_keywords_export_extends() {
    let (set, syn) = setup();
    let tokens = toks("export object X extends Y", &set, &syn);
    for kw in ["export", "object", "extends"] {
        let hit = tokens
            .iter()
            .find(|(s, t)| t == kw && s.contains("keyword.other"))
            .unwrap_or_else(|| panic!("{kw} not marked keyword.other"));
        let _ = hit;
    }
}

#[test]
fn control_keyword_function_not_control() {
    // function 归 keyword.function,不由 control 规则吞掉
    let (set, syn) = setup();
    let tokens = toks("function foo() end", &set, &syn);
    let f = tokens
        .iter()
        .find(|(_, t)| t == "function")
        .expect("function token");
    assert!(f.0.contains("keyword.function"), "scope: {}", f.0);
}