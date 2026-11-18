use std::sync::Arc;

use crate::text::{TextRange, TextSize};

pub type Kind = u16;

pub const KIND_TRIVIA: Kind = 0;
pub const KIND_ERROR: Kind = 1;
pub const KIND_CHUNK: Kind = 2;
pub const KIND_STMT: Kind = 3;
pub const KIND_TOKEN_BASE: Kind = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GreenToken {
    pub kind: Kind,
    pub text: Box<str>,
}

impl GreenToken {
    pub fn len(&self) -> TextSize {
        TextSize(self.text.chars().count() as u32)
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

#[derive(Debug, Clone)]
pub enum GreenChild {
    Token {
        token: Arc<GreenToken>,
        offset: TextSize,
    },
    Node {
        node: Arc<GreenNode>,
        offset: TextSize,
    },
}

impl GreenChild {
    pub fn len(&self) -> TextSize {
        match self {
            GreenChild::Token { token, .. } => token.len(),
            GreenChild::Node { node, .. } => node.len,
        }
    }

    pub fn range(&self, parent_start: TextSize) -> TextRange {
        let offset = match self {
            GreenChild::Token { offset, .. } | GreenChild::Node { offset, .. } => *offset,
        };
        TextRange::from_sizes(parent_start + offset, parent_start + offset + self.len())
    }

    pub fn kind(&self) -> Kind {
        match self {
            GreenChild::Token { token, .. } => token.kind,
            GreenChild::Node { node, .. } => node.kind,
        }
    }

    pub fn as_token(&self) -> Option<&Arc<GreenToken>> {
        match self {
            GreenChild::Token { token, .. } => Some(token),
            GreenChild::Node { .. } => None,
        }
    }

    pub fn as_node(&self) -> Option<&Arc<GreenNode>> {
        match self {
            GreenChild::Node { node, .. } => Some(node),
            GreenChild::Token { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GreenNode {
    pub kind: Kind,
    pub len: TextSize,
    pub children: Box<[GreenChild]>,
}

impl GreenNode {
    pub fn write_text(&self, out: &mut String) {
        for child in &self.children {
            match child {
                GreenChild::Token { token, .. } => out.push_str(&token.text),
                GreenChild::Node { node, .. } => node.write_text(out),
            }
        }
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        self.write_text(&mut out);
        out
    }

    pub fn children(&self) -> &[GreenChild] {
        &self.children
    }
}

struct Frame {
    kind: Kind,
    children: Vec<GreenChild>,
    len: TextSize,
}

pub struct GreenBuilder {
    kind: Kind,
    children: Vec<GreenChild>,
    len: TextSize,
    stack: Vec<Frame>,
}

impl GreenBuilder {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            children: vec![],
            len: TextSize::ZERO,
            stack: vec![],
        }
    }

    pub fn start_node(&mut self, kind: Kind) {
        self.stack.push(Frame {
            kind,
            children: vec![],
            len: TextSize::ZERO,
        });
    }

    pub fn finish_node(&mut self) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        let node = Arc::new(GreenNode {
            kind: frame.kind,
            len: frame.len,
            children: frame.children.into_boxed_slice(),
        });
        let len = node.len;
        let offset = self.current_len();
        self.current_children_mut()
            .push(GreenChild::Node { node, offset });
        *self.current_len_mut() = self.current_len() + len;
    }

    pub fn token(&mut self, kind: Kind, text: impl Into<Box<str>>) {
        let token = Arc::new(GreenToken {
            kind,
            text: text.into(),
        });
        let len = token.len();
        let offset = self.current_len();
        self.current_children_mut()
            .push(GreenChild::Token { token, offset });
        *self.current_len_mut() = self.current_len() + len;
    }

    pub fn reuse_token(&mut self, token: Arc<GreenToken>) {
        let len = token.len();
        let offset = self.current_len();
        self.current_children_mut()
            .push(GreenChild::Token { token, offset });
        *self.current_len_mut() = self.current_len() + len;
    }

    fn current_children_mut(&mut self) -> &mut Vec<GreenChild> {
        match self.stack.last_mut() {
            Some(frame) => &mut frame.children,
            None => &mut self.children,
        }
    }

    fn current_len_mut(&mut self) -> &mut TextSize {
        match self.stack.last_mut() {
            Some(frame) => &mut frame.len,
            None => &mut self.len,
        }
    }

    fn current_len(&self) -> TextSize {
        match self.stack.last() {
            Some(frame) => frame.len,
            None => self.len,
        }
    }

    pub fn finish(mut self) -> Arc<GreenNode> {
        while !self.stack.is_empty() {
            self.finish_node();
        }
        Arc::new(GreenNode {
            kind: self.kind,
            len: self.len,
            children: self.children.into_boxed_slice(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct SyntaxTree {
    pub root: Arc<GreenNode>,
}

impl SyntaxTree {
    pub fn text(&self) -> String {
        self.root.text()
    }

    pub fn text_range(&self) -> TextRange {
        TextRange::from_sizes(TextSize::ZERO, self.root.len)
    }

    pub fn tokens(&self) -> Vec<(Arc<GreenToken>, TextRange)> {
        let mut out = vec![];
        collect_tokens(&self.root, TextSize::ZERO, &mut out);
        out
    }
}

fn collect_tokens(node: &GreenNode, start: TextSize, out: &mut Vec<(Arc<GreenToken>, TextRange)>) {
    for child in &node.children {
        match child {
            GreenChild::Token { token, offset } => {
                let from = start + *offset;
                out.push((
                    token.clone(),
                    TextRange::from_sizes(from, from + token.len()),
                ));
            }
            GreenChild::Node { node, offset } => collect_tokens(node, start + *offset, out),
        }
    }
}

pub enum TokenInput {
    Fresh {
        kind: Kind,
        range: TextRange,
    },
    Reused {
        range: TextRange,
        token: Arc<GreenToken>,
    },
}

fn char_offsets(source: &str) -> Vec<usize> {
    let mut offsets: Vec<usize> = source.char_indices().map(|(index, _)| index).collect();
    offsets.push(source.len());
    offsets
}

fn slice<'a>(source: &'a str, offsets: &[usize], range: TextRange) -> &'a str {
    let from = offsets
        .get(range.start.get() as usize)
        .copied()
        .unwrap_or(source.len());
    let to = offsets
        .get(range.end.get() as usize)
        .copied()
        .unwrap_or(source.len())
        .min(source.len());
    source.get(from..to).unwrap_or("")
}

fn build_tree_with(source: &str, inputs: Vec<TokenInput>) -> SyntaxTree {
    let offsets = char_offsets(source);
    let total = TextSize(offsets.len().saturating_sub(1) as u32);
    let mut sorted = inputs;
    sorted.sort_by_key(|input| match input {
        TokenInput::Fresh { range, .. } | TokenInput::Reused { range, .. } => *range,
    });
    let mut builder = GreenBuilder::new(KIND_CHUNK);
    let mut cursor = TextSize::ZERO;
    for input in sorted {
        let (kind, range, reused) = match input {
            TokenInput::Fresh { kind, range } => (kind, range, None),
            TokenInput::Reused { range, token } => (token.kind, range, Some(token)),
        };
        if range.end <= cursor || range.start >= total {
            continue;
        }
        let start = range.start.max(cursor);
        if start > cursor {
            builder.token(
                KIND_TRIVIA,
                slice(source, &offsets, TextRange::from_sizes(cursor, start)),
            );
        }
        let end = range.end.min(total);
        if let Some(token) = reused {
            builder.reuse_token(token);
        } else {
            builder.token(
                kind,
                slice(source, &offsets, TextRange::from_sizes(start, end)),
            );
        }
        cursor = end;
    }
    if cursor < total {
        builder.token(
            KIND_TRIVIA,
            slice(source, &offsets, TextRange::from_sizes(cursor, total)),
        );
    }
    SyntaxTree {
        root: builder.finish(),
    }
}

pub fn build_tree(source: &str, tokens: &[(Kind, TextRange)]) -> SyntaxTree {
    build_tree_with(
        source,
        tokens
            .iter()
            .map(|(kind, range)| TokenInput::Fresh {
                kind: *kind,
                range: *range,
            })
            .collect(),
    )
}

pub fn rebuild_tree(
    old: &SyntaxTree,
    new_source: &str,
    changed: TextRange,
    new_tokens: &[(Kind, TextRange)],
) -> SyntaxTree {
    let delta = new_source.chars().count() as i64 - old.text_range().end.get() as i64;
    let mut inputs: Vec<TokenInput> = vec![];
    for (token, range) in old.tokens() {
        if token.kind == KIND_TRIVIA || range.intersects(changed) {
            continue;
        }
        let shifted = if range.start >= changed.end {
            range.shifted(delta)
        } else {
            range
        };
        inputs.push(TokenInput::Reused {
            range: shifted,
            token,
        });
    }
    inputs.extend(
        new_tokens
            .iter()
            .filter(|(_, range)| range.intersects(changed))
            .map(|(kind, range)| TokenInput::Fresh {
                kind: *kind,
                range: *range,
            }),
    );
    build_tree_with(new_source, inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range_of(source: &str, needle: &str, nth: usize) -> TextRange {
        let byte = source.match_indices(needle).nth(nth).unwrap().0;
        let start = source[..byte].chars().count() as u32;
        TextRange::new(start, start + needle.chars().count() as u32)
    }

    #[test]
    fn tree_text_is_lossless() {
        let source = "local a = 1  -- note\nreturn a\n";
        let tokens = [
            (100, range_of(source, "local", 0)),
            (101, range_of(source, "a", 0)),
            (102, range_of(source, "=", 0)),
            (103, range_of(source, "1", 0)),
            (104, range_of(source, "return", 0)),
            (105, range_of(source, "a", 1)),
        ];
        assert_eq!(build_tree(source, &tokens).text(), source);
    }

    #[test]
    fn tree_uses_char_offsets_with_unicode() {
        let source = "local s = \"😀\"  -- c\nreturn s\n";
        let tokens = [
            (100, range_of(source, "local", 0)),
            (101, range_of(source, "s", 0)),
            (102, range_of(source, "=", 0)),
            (103, range_of(source, "\"😀\"", 0)),
            (104, range_of(source, "return", 0)),
            (105, range_of(source, "s", 1)),
        ];
        assert_eq!(build_tree(source, &tokens).text(), source);
    }

    #[test]
    fn rebuild_reuses_untouched_tokens() {
        let old = "local a = 1\nlocal b = 2\nreturn b\n";
        let old_tokens = [
            (100, range_of(old, "local", 0)),
            (101, range_of(old, "a", 0)),
            (102, range_of(old, "=", 0)),
            (103, range_of(old, "1", 0)),
            (100, range_of(old, "local", 1)),
            (101, range_of(old, "b", 0)),
            (102, range_of(old, "=", 1)),
            (103, range_of(old, "2", 0)),
            (104, range_of(old, "return", 0)),
            (101, range_of(old, "b", 1)),
        ];
        let tree = build_tree(old, &old_tokens);
        let first_local = tree
            .tokens()
            .into_iter()
            .find(|(token, _)| &*token.text == "local")
            .unwrap()
            .0;

        let new = "local a = 1\nlocal b = 200\nreturn b\n";
        let changed = range_of(old, "2", 0);
        let new_tokens = [
            (100, range_of(new, "local", 0)),
            (101, range_of(new, "a", 0)),
            (102, range_of(new, "=", 0)),
            (103, range_of(new, "1", 0)),
            (100, range_of(new, "local", 1)),
            (101, range_of(new, "b", 0)),
            (102, range_of(new, "=", 1)),
            (103, range_of(new, "200", 0)),
            (104, range_of(new, "return", 0)),
            (101, range_of(new, "b", 1)),
        ];
        let rebuilt = rebuild_tree(&tree, new, changed, &new_tokens);
        assert_eq!(rebuilt.text(), new);
        let kept = rebuilt
            .tokens()
            .into_iter()
            .find(|(token, _)| &*token.text == "local")
            .unwrap()
            .0;
        assert!(Arc::ptr_eq(&first_local, &kept));
    }

    #[test]
    fn green_builder_nests_nodes_with_correct_ranges() {
        let mut builder = GreenBuilder::new(KIND_CHUNK);
        builder.token(100, "abc");
        builder.start_node(200);
        builder.token(101, "de");
        builder.token(102, "f");
        builder.finish_node();
        let root = builder.finish();
        assert_eq!(root.text(), "abcdef");
        let nested = root.children()[1].as_node().unwrap();
        assert_eq!(nested.text(), "def");
        assert_eq!(nested.len, TextSize(3));
        assert_eq!(
            root.children()[1].range(TextSize::ZERO),
            TextRange::new(3, 6)
        );
    }
}
