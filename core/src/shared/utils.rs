use std::{collections::VecDeque, iter::Fuse};

// #[derive(Debug, Clone, Copy)]
// pub struct Bits<const L: u8, const S: bool>(pub u32);
// impl<const L: u8> From<u32> for Bits<L, false> {
//     fn from(value: u32) -> Self {
//         Self(value & (2u32.pow(L as u32) - 1))
//     }
// }
// impl<const L: u8> From<u32> for Bits<L, true> {
//     fn from(value: u32) -> Self {
//         Self(value & (2u32.pow(L as u32 - 1) - 1))
//     }
// }
// impl<const L: u8> From<i32> for Bits<L, true> {
//     fn from(value: i32) -> Self {
//         let mask = (1 << (L - 1)) - 1;
//         let abs = value.unsigned_abs() as u32;
//         if value < 0 {
//             Self((abs & mask) | (1 << (L - 1)))
//         } else {
//             Self(abs & mask)
//         }
//     }
// }
// impl<const L: u8> From<Bits<L, false>> for u32 {
//     fn from(value: Bits<L, false>) -> Self {
//         value.0
//     }
// }
// impl<const L: u8> From<Bits<L, true>> for i32 {
//     fn from(value: Bits<L, true>) -> Self {
//         let val = (value.0 & (2u32.pow(L as u32 - 1) - 1)) as i32;
//         if value.0 & (1 << (L - 1)) != 0 {
//             -val
//         } else {
//             val
//         }
//     }
// }

/// When returning value does not depent on whether it was success
#[derive(Debug)]
pub enum Action<T> {
    Success(T),
    Failure(T),
}

pub type TryDo<T, E> = Result<Option<T>, E>;

/// NO LONGER NEEDED...
/// I'm looking forward to
///
/// Things will come true or stay in imagination, or there is something wrong happened in the world
// pub type Expect<E> = Result<bool, E>;

/// An iterator with a `peek_nth()` that returns an optional reference to the next nth
/// element.
///
/// ## DO *NOT* USE OTHER ITERATOR FUNCTIONS EXCEPT next AND count
///
/// This `struct` is created by the [`multi_peekable`] method on [`Iterator`]
///
/// [`multi_peekable`]: MultiPeekableExtension::multi_peekable
/// [`Iterator`]: trait.Iterator.html
#[derive(Debug, Clone)]
pub struct MultiPeekable<I>
where
    I: Iterator,
{
    iter: Fuse<I>,
    buf: VecDeque<I::Item>,
}

impl<I: Iterator> MultiPeekable<I> {
    pub const MAX_DEPTH: usize = 3;

    #[inline]
    fn new(iter: I) -> Self {
        Self {
            iter: iter.fuse(),
            buf: VecDeque::new(),
        }
    }

    /// ## `n` must be less than `MAX_DEPTH`
    pub fn peek_nth(&mut self, n: usize) -> Option<&<I as Iterator>::Item> {
        if n > Self::MAX_DEPTH {
            return None;
        }

        while self.buf.len() <= n {
            match self.iter.next() {
                Some(item) => self.buf.push_back(item),
                None => break,
            }
        }

        self.buf.get(n)
    }

    /*
    pub fn peek(&mut self) -> Option<&<I as Iterator>::Item> {
        self.peek_nth(0)
    }
    */
}
impl<I: Iterator> Iterator for MultiPeekable<I> {
    type Item = I::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.buf.pop_front() {
            Some(item)
        } else {
            self.iter.next()
        }
    }

    #[inline]
    fn count(self) -> usize {
        if self.buf.len() != 0 {
            self.buf.len() + self.iter.count()
        } else {
            self.iter.count()
        }
    }
}

pub trait MultiPeekableExtension: Iterator + Sized {
    fn multi_peekable(self) -> MultiPeekable<Self>;
}
impl<I: Iterator> MultiPeekableExtension for I {
    fn multi_peekable(self) -> MultiPeekable<Self> {
        MultiPeekable::new(self)
    }
}

#[inline]
pub const fn is_newline(input: u8) -> bool {
    input == b'\n' || input == b'\r'
}

#[inline]
pub const fn is_valid_radix(input: u8, radix: u32) -> bool {
    match radix {
        2 => input == b'0' || input == b'1',
        8 => b'0' <= input && input <= b'7',
        10 => input.is_ascii_digit(),
        16 => input.is_ascii_hexdigit(),
        _ => false,
    }
}

#[inline]
pub const fn get_radix(b: u8) -> Option<u32> {
    match b.to_ascii_lowercase() {
        b'b' => Some(2),
        b'o' => Some(8),
        b'x' => Some(16),
        _ => None,
    }
}

/// we must ensure that all of the input are valid utf8
#[inline]
pub const fn len_utf8_by_head(head: u8) -> u8 {
    match head {
        // 110xxxxx
        0xC0..=0xDF => 2,
        // 1110xxxx
        0xE0..=0xEF => 3,
        // 11110xxx
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

#[inline]
pub const fn check_utf8_head(head: u8) -> bool {
    head <= 0b11110111
}

#[inline]
pub const fn check_utf8_body(body: u8) -> bool {
    body & 0b10000000 != 0
}

/// convert u32 to utf8 bytes, write into vec
///
/// we must ensure that code are valid unicode
#[inline]
pub fn encode_utf8_bytes(code: u32, v: &mut Vec<u8>) {
    match code {
        // 一字节
        // 原样放入
        0x00..=0x7F => v.push(code as u8),
        // 两字节
        // code的二进制数字有8~11位
        0x80..=0x7FF => {
            // >> 6 先取前2~5位
            // 0xC0 | ~ 加上前缀11
            v.push(0xC0 | (code >> 6) as u8);
            // & 0x3F 再取出后6位 位掩码用于提取特定位
            // 0x80 | ~ 加上前缀10
            v.push(0x80 | (code & 0x3F) as u8);
        }
        // 三字节
        0x800..=0xFFFF => {
            v.push(0xE0 | (code >> 12) as u8);
            v.push(0x80 | ((code >> 6) & 0x3F) as u8);
            v.push(0x80 | (code & 0x3F) as u8);
        }
        // 四字节
        0x10000..=0x10FFFF => {
            v.push(0xF0 | (code >> 18) as u8);
            v.push(0x80 | ((code >> 12) & 0x3F) as u8);
            v.push(0x80 | ((code >> 6) & 0x3F) as u8);
            v.push(0x80 | (code & 0x3F) as u8);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub const fn is_valid_ident(b: u8, head: bool) -> bool {
    b.is_ascii_alphabetic() || (b.is_ascii_digit() && !head) || b >= 127 || b == b'_'
}
