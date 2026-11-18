use std::collections::HashMap;

use duka_shared::{
    dtype::{Kind, KindShape, Ty},
    effects::EffectSet,
    errors::Span,
    types::{BinOp, LogicDatabase, Pipeline, SourceInfo, SysCall, UnOp},
    value::{AttrValue, ConstValue},
};

use crate::lexer::token::TokenKind;

pub trait SpanMut {
    fn span_mut(&mut self, f: &mut dyn FnMut(&mut Span));
}

pub fn remap<T: SpanMut>(value: &mut T, mut map: impl FnMut(&Span) -> Span) {
    value.span_mut(&mut |span| {
        *span = map(span);
    });
}

impl SpanMut for Span {
    fn span_mut(&mut self, f: &mut dyn FnMut(&mut Span)) {
        f(self);
    }
}

impl<T> SpanMut for Option<T>
where
    T: SpanMut,
{
    fn span_mut(&mut self, f: &mut dyn FnMut(&mut Span)) {
        if let Some(value) = self {
            value.span_mut(f);
        }
    }
}

impl<T> SpanMut for Box<T>
where
    T: SpanMut + ?Sized,
{
    fn span_mut(&mut self, f: &mut dyn FnMut(&mut Span)) {
        (**self).span_mut(f);
    }
}

impl<T> SpanMut for Vec<T>
where
    T: SpanMut,
{
    fn span_mut(&mut self, f: &mut dyn FnMut(&mut Span)) {
        for value in self {
            value.span_mut(f);
        }
    }
}

impl<T> SpanMut for Box<[T]>
where
    T: SpanMut,
{
    fn span_mut(&mut self, f: &mut dyn FnMut(&mut Span)) {
        for value in self.iter_mut() {
            value.span_mut(f);
        }
    }
}

impl<K, V> SpanMut for HashMap<K, V>
where
    V: SpanMut,
{
    fn span_mut(&mut self, f: &mut dyn FnMut(&mut Span)) {
        for value in self.values_mut() {
            value.span_mut(f);
        }
    }
}

impl<A, B> SpanMut for (A, B)
where
    A: SpanMut,
    B: SpanMut,
{
    fn span_mut(&mut self, f: &mut dyn FnMut(&mut Span)) {
        self.0.span_mut(f);
        self.1.span_mut(f);
    }
}

impl<A, B, C> SpanMut for (A, B, C)
where
    A: SpanMut,
    B: SpanMut,
    C: SpanMut,
{
    fn span_mut(&mut self, f: &mut dyn FnMut(&mut Span)) {
        self.0.span_mut(f);
        self.1.span_mut(f);
        self.2.span_mut(f);
    }
}

impl<L> SpanMut for Kind<L> {
    fn span_mut(&mut self, _f: &mut dyn FnMut(&mut Span)) {}
}

impl<L> SpanMut for Ty<L> {
    fn span_mut(&mut self, _f: &mut dyn FnMut(&mut Span)) {}
}

macro_rules! noop_span {
    ($($ty:ty),* $(,)?) => {
        $(impl SpanMut for $ty {
            fn span_mut(&mut self, _f: &mut dyn FnMut(&mut Span)) {}
        })*
    };
}

noop_span!(
    TokenKind,
    KindShape,
    EffectSet,
    SourceInfo,
    LogicDatabase,
    BinOp,
    Pipeline,
    SysCall,
    UnOp,
    ConstValue,
    AttrValue,
    u8,
    u16,
    u32,
    u64,
    usize,
    i8,
    i16,
    i32,
    i64,
    isize,
    f32,
    f64,
    bool,
    char,
    String,
    Box<str>,
    std::sync::Arc<str>,
);
