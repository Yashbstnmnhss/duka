use std::any::TypeId;

use crate::{Trace, Tracer};

#[repr(u8)]
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum GcColor {
    White, // 0b00
    Gray,  // 0b01
    Black, // 0b10
    Dead,  // 0b11
}
impl GcColor {
    /// # Panic
    /// When no such variant
    pub(crate) fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::White,
            1 => Self::Gray,
            2 => Self::Black,
            3 => Self::Dead,
            _ => panic!("No such variant"),
        }
    }
}
pub const COLOR_BITS: u8 = 2;
pub const COLOR_MASK: u8 = 2u8.pow(COLOR_BITS as u32) - 1;

#[repr(u8)]
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum GcAge {
    New,
    Survival,
    Old,
}
impl GcAge {
    /// # Panic
    /// When no such variant
    pub(crate) fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::New,
            1 => Self::Survival,
            2 => Self::Old,
            _ => panic!("No such variant"),
        }
    }
}
pub const AGE_BITS: u8 = 2;
pub const AGE_MASK: u8 = (2u8.pow(AGE_BITS as u32) - 1) << COLOR_BITS;

#[repr(u8)]
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum GcFlag {
    Finalizable = 0b001,
    Reachable = 0b010,
    Marked = 0b100,
}

/// Drop `Box<T>` from its pointer
///
/// # Safety
/// `p` must be a valid pointer from `Box::into_raw`
#[inline]
pub unsafe fn drop_box<T>(p: *mut u8) {
    unsafe {
        let tptr = p as *mut T;
        drop(Box::from_raw(tptr))
    }
}

pub type TraceFn = unsafe fn(*mut u8, &mut Tracer);
#[inline]
fn trace_fn<T: Trace>() -> TraceFn {
    |obj_ptr, tracer| unsafe {
        let obj = &*(obj_ptr as *const T);
        obj.trace(tracer);
    }
}

//#[repr(C)]
pub struct GcHeader {
    /// Header info
    info: u8, // 1 byte

    // 以结构体内的字段的最大对齐方式(align = 0x8)为对齐
    // 由于repr(C)和需要用裸指针读取header
    //_pad: [u8; 7], //+7 bytes  => 8 bytes
    /// NOTE(TODO): Layout of TypeId is unstable, see [`TypeId`]
    ///
    /// 类型信息
    pub type_id: TypeId,
    /// 解构函数的指针
    pub destructor: unsafe fn(*mut u8),
    pub trace_fn: TraceFn,
}
impl GcHeader {
    /// Get header from a pointer of object
    ///
    /// # Safety
    /// `obj_ptr` must points to a object allocated by current allocator
    #[inline]
    pub unsafe fn from_obj_ptr<T>(obj_ptr: *const T) -> &'static mut Self {
        let raw = obj_ptr as *mut u8;
        let header_ptr = unsafe { raw.sub(size_of::<Self>()) } as *mut Self;
        unsafe { &mut *header_ptr }
    }

    pub fn init<T: 'static + Trace>(&mut self) {
        self.info = GcColor::White as u8;
        self.type_id = TypeId::of::<T>();
        self.destructor = drop_box::<T>;
        self.trace_fn = trace_fn::<T>();
    }

    #[inline]
    pub fn get_age(&self) -> GcAge {
        GcAge::from_u8((self.info & AGE_MASK) >> COLOR_BITS)
    }
    #[inline]
    pub fn get_color(&self) -> GcColor {
        GcColor::from_u8(self.info & COLOR_MASK)
    }
    #[inline]
    pub fn set_age(&mut self, age: GcAge) {
        self.info = (self.info & !AGE_MASK) | ((age as u8) << COLOR_BITS);
    }
    #[inline]
    pub fn set_color(&mut self, color: GcColor) {
        self.info = (self.info & !COLOR_MASK) | (color as u8);
    }
    #[inline]
    pub fn set_flag(&mut self, flag: GcFlag) {
        let flag = (flag as u8) << (COLOR_BITS + AGE_BITS);
        self.info = (self.info & !flag) | flag;
    }
    #[inline]
    pub fn has_flag(&self, flag: GcFlag) -> bool {
        let flag = (flag as u8) << (COLOR_BITS + AGE_BITS);
        self.info & flag != 0
    }
    #[inline]
    pub fn remove_flag(&mut self, flag: GcFlag) {
        let flag = (flag as u8) << (COLOR_BITS + AGE_BITS);
        self.info = self.info & !flag;
    }
    #[inline]
    pub const fn total_size<T>() -> usize {
        size_of::<Self>() + size_of::<T>()
    }
}
