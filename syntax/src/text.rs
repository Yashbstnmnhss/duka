use std::ops::{Add, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TextSize(pub u32);

impl TextSize {
    pub const ZERO: Self = Self(0);

    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl Add for TextSize {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for TextSize {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TextRange {
    pub start: TextSize,
    pub end: TextSize,
}

impl TextRange {
    pub const fn new(start: u32, end: u32) -> Self {
        Self {
            start: TextSize(start),
            end: TextSize(end),
        }
    }

    pub const fn from_sizes(start: TextSize, end: TextSize) -> Self {
        Self { start, end }
    }

    pub const fn len(self) -> TextSize {
        TextSize(self.end.0 - self.start.0)
    }

    pub const fn is_empty(self) -> bool {
        self.start.0 == self.end.0
    }

    pub const fn contains(self, at: TextSize) -> bool {
        at.0 >= self.start.0 && at.0 < self.end.0
    }

    pub const fn contains_range(self, other: Self) -> bool {
        other.start.0 >= self.start.0 && other.end.0 <= self.end.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.start.0 < other.end.0 && other.start.0 < self.end.0
    }

    pub const fn shifted(self, by: i64) -> Self {
        Self {
            start: TextSize((self.start.0 as i64 + by) as u32),
            end: TextSize((self.end.0 as i64 + by) as u32),
        }
    }
}
