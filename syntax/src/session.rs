use std::sync::Arc;

use crate::green::{GreenBuilder, KIND_CHUNK, KIND_TRIVIA, SyntaxTree};
use crate::text::TextRange;

#[derive(Debug, Clone)]
pub struct Edit {
    pub range: TextRange,
    pub text: String,
}

impl Edit {
    pub fn new(start: u32, end: u32, text: impl Into<String>) -> Self {
        Self {
            range: TextRange::new(start, end),
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewToken {
    pub kind: u16,
    pub range: TextRange,
    pub text: String,
}

impl NewToken {
    pub fn new(kind: u16, start: u32, end: u32, text: impl Into<String>) -> Self {
        Self {
            kind,
            range: TextRange::new(start, end),
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum TreeEvent {
    Token(NewToken),
    StartNode(u16),
    FinishNode,
}

#[derive(Default)]
pub struct SyntaxSession {
    tree: Option<SyntaxTree>,
}

impl SyntaxSession {
    pub fn new() -> Self {
        Self { tree: None }
    }

    pub fn with_tree(mut self, tree: SyntaxTree) -> Self {
        self.tree = Some(tree);
        self
    }

    pub fn tree(&self) -> Option<&SyntaxTree> {
        self.tree.as_ref()
    }

    pub fn set_tree(&mut self, tree: SyntaxTree) {
        self.tree = Some(tree);
    }

    pub fn build(self, builder: GreenBuilder) -> SyntaxTree {
        SyntaxTree {
            root: builder.finish(),
        }
    }

    pub fn reparse(&mut self, source: &str, events: &[TreeEvent]) -> SyntaxTree {
        let old = self.tree.take();
        let tree = match old {
            Some(old) => self.reuse_statements(&old, source, events),
            None => build_full(source, events),
        };
        self.tree = Some(tree.clone());
        tree
    }

    fn reuse_statements(&self, old: &SyntaxTree, source: &str, events: &[TreeEvent]) -> SyntaxTree {
        let _ = source;
        let old_stmts = old.top_statements();
        let groups = group_statements(events);
        let mut builder = GreenBuilder::new(KIND_CHUNK);
        let mut old_index = 0usize;

        for group in groups {
            let group_tokens: Vec<_> = group
                .iter()
                .filter_map(|event| match event {
                    TreeEvent::Token(token) if token.kind != KIND_TRIVIA => Some(token),
                    _ => None,
                })
                .collect();
            if group_tokens.is_empty() {
                for event in &group {
                    match event {
                        TreeEvent::StartNode(kind) => builder.start_node(*kind),
                        TreeEvent::FinishNode => builder.finish_node(),
                        TreeEvent::Token(token) => builder.token(token.kind, token.text.as_str()),
                    }
                }
                continue;
            }
            let reused = old_stmts.get(old_index).filter(|stmt| {
                let leaves = stmt.leaf_tokens();
                leaves.len() == group_tokens.len()
                    && leaves.iter().zip(group_tokens.iter()).all(|(old, new)| {
                        old.kind == new.kind && old.text.as_ref() == new.text.as_str()
                    })
            });
            if let Some(stmt) = reused {
                builder.reuse_node(Arc::clone(stmt));
                old_index += 1;
                continue;
            }
            if old_stmts.get(old_index).is_some() {
                old_index += 1;
            }
            for event in &group {
                match event {
                    TreeEvent::StartNode(kind) => builder.start_node(*kind),
                    TreeEvent::FinishNode => builder.finish_node(),
                    TreeEvent::Token(token) => builder.token(token.kind, token.text.as_str()),
                }
            }
        }
        SyntaxTree {
            root: builder.finish(),
        }
    }
}

fn group_statements(events: &[TreeEvent]) -> Vec<Vec<TreeEvent>> {
    let mut groups = vec![];
    let mut current: Vec<TreeEvent> = vec![];
    let mut depth = 0usize;
    for event in events {
        match event {
            TreeEvent::StartNode(_) => {
                if depth == 0 && !current.is_empty() {
                    groups.push(std::mem::take(&mut current));
                }
                depth += 1;
                current.push(event.clone());
            }
            TreeEvent::FinishNode => {
                current.push(event.clone());
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    groups.push(std::mem::take(&mut current));
                }
            }
            TreeEvent::Token(token) => {
                if depth == 0 {
                    groups.push(vec![TreeEvent::Token(token.clone())]);
                } else {
                    current.push(event.clone());
                }
            }
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn build_full(source: &str, events: &[TreeEvent]) -> SyntaxTree {
    let _ = source;
    let mut builder = GreenBuilder::new(KIND_CHUNK);
    for event in events {
        match event {
            TreeEvent::StartNode(kind) => builder.start_node(*kind),
            TreeEvent::FinishNode => builder.finish_node(),
            TreeEvent::Token(token) => builder.token(token.kind, token.text.as_str()),
        }
    }
    SyntaxTree {
        root: builder.finish(),
    }
}

#[cfg(test)]
fn slice_text(text: &str, from: u32, to: u32) -> String {
    let offsets = char_offsets(text);
    let from = offsets.get(from as usize).copied().unwrap_or(text.len());
    let to = offsets
        .get(to as usize)
        .copied()
        .unwrap_or(text.len())
        .min(text.len());
    text.get(from..to).unwrap_or("").to_owned()
}

#[cfg(test)]
fn char_offsets(source: &str) -> Vec<usize> {
    let mut offsets: Vec<usize> = source.char_indices().map(|(index, _)| index).collect();
    offsets.push(source.len());
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::green::KIND_STMT;

    fn tokens(source: &str) -> Vec<NewToken> {
        let mut out = vec![];
        let mut cursor = 0usize;
        let bytes = source.as_bytes();
        let mut word_start: Option<usize> = None;
        for (index, byte) in bytes.iter().enumerate() {
            let is_word = byte.is_ascii_alphanumeric() || *byte == b'_';
            match (word_start, is_word) {
                (None, true) => word_start = Some(index),
                (Some(_), false) => {
                    let end = index;
                    let start = word_start.take().unwrap();
                    push_word(&mut out, source, start, end);
                }
                _ => {}
            }
            if *byte == b'=' {
                if let Some(start) = word_start.take() {
                    push_word(&mut out, source, start, index);
                }
                let s = source[..index].chars().count() as u32;
                out.push(NewToken::new(101, s, s + 1, "="));
                cursor = index + 1;
            }
        }
        if let Some(start) = word_start {
            push_word(&mut out, source, start, bytes.len());
        }
        let _ = cursor;
        out.sort_by_key(|token| (token.range.start, token.range.end));
        out
    }

    fn push_word(out: &mut Vec<NewToken>, source: &str, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let s = source[..start].chars().count() as u32;
        let e = s + source[start..end].chars().count() as u32;
        out.push(NewToken::new(100, s, e, &source[start..end]));
    }

    fn make_events(source: &str) -> Vec<TreeEvent> {
        let toks = tokens(source);
        let total = source.chars().count() as u32;
        let mut events = vec![];
        let mut cursor = 0u32;
        for token in toks {
            if token.range.start.get() > cursor {
                events.push(TreeEvent::Token(NewToken::new(
                    KIND_TRIVIA,
                    cursor,
                    token.range.start.get(),
                    slice_text(source, cursor, token.range.start.get()),
                )));
            }
            events.push(TreeEvent::StartNode(KIND_STMT));
            events.push(TreeEvent::Token(token.clone()));
            events.push(TreeEvent::FinishNode);
            cursor = token.range.end.get();
        }
        if cursor < total {
            events.push(TreeEvent::Token(NewToken::new(
                KIND_TRIVIA,
                cursor,
                total,
                slice_text(source, cursor, total),
            )));
        }
        events
    }

    #[test]
    fn reparse_keeps_text_lossless() {
        let old = "local a = 1\nlocal b = 2\nreturn b\n";
        let tree = build_full(old, &make_events(old));
        assert_eq!(tree.text(), old);

        let new = "local a = 1\nlocal b = 200\nreturn b\n";
        let mut session = SyntaxSession::new().with_tree(tree);
        let rebuilt = session.reparse(new, &make_events(new));
        assert_eq!(rebuilt.text(), new);
    }

    #[test]
    fn reparse_reuses_unaffected_statement_nodes() {
        let old = "local a = 1\nlocal b = 2\nreturn b\n";
        let tree = build_full(old, &make_events(old));
        let first = Arc::clone(&tree.top_statements()[0]);

        let new = "local a = 1\nlocal b = 200\nreturn b\n";
        let mut session = SyntaxSession::new().with_tree(tree);
        let rebuilt = session.reparse(new, &make_events(new));

        let kept = rebuilt.top_statements().into_iter().next().unwrap();
        assert!(
            Arc::ptr_eq(&first, &kept),
            "the first statement must be reused by Arc identity"
        );
    }
}
