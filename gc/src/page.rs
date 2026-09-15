use std::alloc::{Layout, alloc, handle_alloc_error};

use crate::header::GcHeader;

pub const PAGE_SIZE: usize = 16 * 1024;

/// 根据数据大小类别分级存储
pub const SIZE_CLASSES: &[usize] = &[
    16,  // UpValue
    32,  // MediumString
    48,  // Table (without metatable)
    64,  // Table (with metatable)
    96,  // Closure & UserData
    128, // Array
    256, // HeapString
    512, // Big Object
];

/// Get the slot size of given class index
/// # Panic
/// When class doesn't exist
pub const fn slot_size(class: usize) -> usize {
    let obj_size = SIZE_CLASSES[class];
    let total = size_of::<GcHeader>() + obj_size;
    (total + 7) & !7
}

/// Find a suitable class for requested size
pub fn size_class_index(requested: usize) -> Option<usize> {
    SIZE_CLASSES
        .iter()
        .enumerate()
        .find_map(|(i, &size)| (requested <= size).then_some(i))
}

#[derive(Debug)]
struct BitMap {
    inner: Vec<usize>,
}
impl BitMap {
    pub fn new() -> Self {
        Self { inner: vec![0; 1] }
    }
    #[inline]
    const fn to_idx(at: usize) -> usize {
        at / size_of::<usize>()
    }
    #[inline]
    const fn to_pos(at: usize) -> usize {
        at % size_of::<usize>()
    }
    pub fn get(&self, at: usize) -> Option<bool> {
        let idx = Self::to_idx(at);
        if idx >= self.inner.len() {
            return None;
        }
        let pos = Self::to_pos(at);
        let mask = 1usize << pos;
        Some((self.inner[idx] & mask) >> pos != 0)
    }
    pub fn set(&mut self, at: usize, val: bool) {
        let idx = Self::to_idx(at);
        if idx >= self.inner.len() {
            for _ in 0..(idx - self.inner.len() + 1) {
                self.inner.push(0);
            }
        }
        let pos = Self::to_pos(at);
        let mask = 1usize << pos;
        self.inner[idx] = (self.inner[idx] & !mask) | (val as usize) << pos;
    }
}

#[derive(Debug)]
/// 内存页
pub struct Page {
    // Raw
    ptr: *mut u8,

    slot_bytes: usize,
    slot_count: usize,

    used: u16,
    /// first available slot index
    free_head: usize,
    allocated: BitMap,
}

impl Page {
    /// # Panic
    /// When class doesn't exist or memory layout is invalid
    pub fn new(class: usize) -> Self {
        let slot_bytes = slot_size(class);
        let slot_count = PAGE_SIZE / slot_bytes;

        let layout = Layout::from_size_align(PAGE_SIZE, 8).expect("Invalid layout");
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            handle_alloc_error(layout);
        }

        for i in 0..slot_count {
            let slot_ptr = unsafe { ptr.add(i * slot_bytes) };
            let next = if i + 1 < slot_count {
                i + 1
            } else {
                usize::MAX
            };
            unsafe {
                // 空闲slot存储下一个slot的索引
                (slot_ptr as *mut usize).write(next);
            }
        }

        Self {
            ptr,
            slot_bytes,
            slot_count,
            used: 0,
            free_head: 0,
            allocated: BitMap::new(),
        }
    }

    /// Allocate a slot from free list, return pointer of its object (GcHeader skipped)
    pub fn alloc(&mut self) -> Option<*mut u8> {
        if self.free_head == usize::MAX {
            return None; // Page is full
        }
        let idx = self.free_head;
        self.allocated.set(idx, true);
        let ptr = unsafe { self.ptr.add(idx * self.slot_bytes) };
        let next = unsafe { (ptr as *mut usize).read() };
        self.free_head = next;
        self.used += 1;

        let ptr = unsafe { ptr.add(size_of::<GcHeader>()) };
        Some(ptr)
    }

    /// # Safety
    /// `obj_ptr` must be a valid pointer pointing to object in current page
    pub unsafe fn dealloc(&mut self, obj_ptr: *mut u8) {
        let ptr = unsafe { obj_ptr.sub(size_of::<GcHeader>()) };
        let idx = (ptr as usize - self.ptr as usize) / self.slot_bytes;
        unsafe {
            (ptr as *mut usize).write(self.free_head);
        }
        self.allocated.set(idx, false);
        self.free_head = idx;
        self.used -= 1;
    }

    /// # Safety
    /// `obj_ptr` must be a valid pointer pointing to object in current page
    pub unsafe fn header_of(&self, obj_ptr: *mut u8) -> &mut GcHeader {
        let ptr = unsafe { obj_ptr.sub(size_of::<GcHeader>()) };
        unsafe { &mut *(ptr as *mut GcHeader) }
    }

    pub fn contains(&self, ptr: *const u8) -> bool {
        let addr = ptr as usize;
        let start = self.ptr as usize;
        addr >= start && addr < start + PAGE_SIZE
    }
    pub fn is_full(&self) -> bool {
        self.free_head == usize::MAX
    }
    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    pub fn iter_allocated(&self) -> PageIter<'_> {
        PageIter {
            page: self,
            current: 0,
        }
    }
}

pub struct PageIter<'a> {
    page: &'a Page,
    current: usize,
}
impl<'a> Iterator for PageIter<'a> {
    type Item = (*mut GcHeader, *mut u8);
    fn next(&mut self) -> Option<Self::Item> {
        while self.current < self.page.slot_count {
            let idx = self.current;
            let header_ptr = unsafe { self.page.ptr.add(self.current * self.page.slot_bytes) };
            let obj_ptr = unsafe { header_ptr.add(size_of::<GcHeader>()) };
            self.current += 1;

            if self.page.allocated.get(idx).unwrap_or_default() {
                return Some((header_ptr as *mut GcHeader, obj_ptr));
            }
        }
        None
    }
}
