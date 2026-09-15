//! GC
use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::any::TypeId;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::header::{GcColor, drop_box};
use crate::{
    header::GcHeader,
    page::{Page, SIZE_CLASSES, size_class_index},
};

pub mod header;
pub mod page;

pub mod prelude {
    pub use super::{Finalize, Gc, GcCell, GcCellRef, GcCellRefMut, Heap, Trace, Tracer};
}

pub trait Trace {
    fn trace(&self, _tracer: &mut Tracer) {}
}

impl<T: Trace> Trace for Option<T> {
    fn trace(&self, tracer: &mut Tracer) {
        if let Some(inner) = self {
            inner.trace(tracer);
        }
    }
}

pub trait Finalize {
    fn finalize(&self) {}
}
pub struct Tracer<'a> {
    pub heap: &'a mut Heap,
}
impl<'a> Tracer<'a> {
    pub fn mark<T: Trace>(&mut self, gc: &Gc<T>) {
        let header = gc.header();
        if header.get_color() == GcColor::White {
            header.set_color(GcColor::Gray);
            self.heap.gray_list.push(gc.ptr.as_ptr() as *mut u8);
        }
    }
}

pub struct Gc<T> {
    ptr: NonNull<u8>,
    _marker: PhantomData<T>,
}
impl<T> Copy for Gc<T> {}
impl<T> Clone for Gc<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for Gc<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }
}
impl<T> Eq for Gc<T> {}
impl<T> std::hash::Hash for Gc<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.ptr.as_ptr() as usize).hash(state)
    }
}
impl<T> std::fmt::Debug for Gc<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Gc({:p})", self.ptr.as_ptr())
    }
}
impl<T> Gc<T> {
    pub fn as_ptr(&self) -> *const () {
        self.ptr.as_ptr() as *const ()
    }

    /// # Safety
    /// Object alive
    #[inline]
    pub fn header(&self) -> &GcHeader {
        unsafe { GcHeader::from_obj_ptr(self.ptr.as_ptr() as *const T) }
    }
}

impl<T> std::ops::Deref for Gc<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        let p = self.ptr.as_ptr() as *const T;
        unsafe { &*p }
    }
}

#[derive(Debug)]
pub struct GcCell<T> {
    inner: UnsafeCell<T>,
}
impl<T> GcCell<T> {
    pub fn new(val: T) -> Self {
        Self {
            inner: UnsafeCell::new(val),
        }
    }

    /// # Safety
    /// 调用时不得存在同一 `GcCell` 的其他可变引用
    pub unsafe fn get(&self) -> &T {
        unsafe { &*self.inner.get() }
    }
}

impl<T: Trace> Trace for GcCell<T> {
    fn trace(&self, tracer: &mut Tracer) {
        let inner_ref = unsafe { &*self.inner.get() };
        inner_ref.trace(tracer);
    }
}

impl<T: Finalize> Finalize for GcCell<T> {
    fn finalize(&self) {
        let inner_ref = unsafe { &*self.inner.get() };
        inner_ref.finalize();
    }
}

pub struct GcCellRef<'a, T> {
    inner: &'a GcCell<T>,
}
impl<'a, T> std::ops::Deref for GcCellRef<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner.inner.get() }
    }
}

pub struct GcCellRefMut<'a, T> {
    inner: &'a GcCell<T>,
}
impl<'a, T> std::ops::Deref for GcCellRefMut<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner.inner.get() }
    }
}
impl<'a, T> std::ops::DerefMut for GcCellRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.inner.inner.get() }
    }
}

impl<T> Gc<GcCell<T>> {
    pub fn borrow(&self) -> GcCellRef<'_, T> {
        let cell_ptr = self.ptr.as_ptr() as *const GcCell<T>;
        let cell_ref = unsafe { &*cell_ptr };
        GcCellRef { inner: cell_ref }
    }
    pub fn borrow_mut(&self) -> GcCellRefMut<'_, T> {
        let cell_ptr = self.ptr.as_ptr() as *const GcCell<T>;
        let cell_ref = unsafe { &*cell_ptr };
        GcCellRefMut { inner: cell_ref }
    }
}

#[derive(Debug)]
struct LargeObject(*mut u8, Layout, TypeId, unsafe fn(*mut u8));

#[derive(Debug)]
pub struct Heap {
    pub gray_list: Vec<*mut u8>,

    /// List of pages by size classes
    page_map: Vec<Vec<Box<Page>>>,

    // TODO
    /// `(class_index, page_index)` for page_map
    #[allow(unused)]
    free_pages: Vec<(usize, usize)>,
    total_objects: usize,
    /// Threshold to trigger GC
    threshold: usize,
    /// Dynamic threshold for next GC
    next_gc: usize,
    /// Big objects (larger than max size class)
    larges: Vec<LargeObject>,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    pub fn new() -> Self {
        let mut page_map = Vec::with_capacity(SIZE_CLASSES.len());
        for _ in 0..SIZE_CLASSES.len() {
            page_map.push(vec![]);
        } // No clone for Page, No vec![vec![]; len]...
        Heap {
            gray_list: vec![],

            page_map,
            free_pages: vec![],
            total_objects: 0,
            threshold: 256,
            next_gc: 256,
            larges: vec![],
        }
    }

    #[inline]
    /// 检查是否需要触发 GC
    pub fn should_collect(&self) -> bool {
        self.total_objects >= self.next_gc
    }

    #[inline]
    /// 设置 GC 触发阈值
    pub fn set_threshold(&mut self, threshold: usize) {
        self.threshold = threshold;
        self.next_gc = threshold;
    }

    #[inline]
    /// 获取当前分配数
    pub fn allocation_count(&self) -> usize {
        self.total_objects
    }

    #[allow(unused)]
    /// 写屏障 TODO
    fn write_barrier(&mut self, obj_ptr: *const u8) {
        unsafe {
            let header = GcHeader::from_obj_ptr(obj_ptr);
            if header.get_color() == GcColor::White {
                header.set_color(GcColor::Gray);
                self.gray_list.push(obj_ptr as *mut u8);
            }
        }
    }

    /// # Panic
    /// When class doesn't exist or memory layout is invalid
    fn find_page_for_class(&mut self, class: usize) -> &mut Page {
        if class >= SIZE_CLASSES.len() {
            panic!("Invalid class")
        }
        let pages = &mut self.page_map[class];
        if let Some(idx) = pages.iter().position(|p| !p.is_full()) {
            return &mut pages[idx];
        }
        let new_page = Box::new(Page::new(class));
        pages.push(new_page);
        pages.last_mut().expect("Checked")
    }

    pub fn alloc_large<T: Trace + 'static>(&mut self, val: T) -> Gc<T> {
        let total_size = GcHeader::total_size::<T>();
        let layout = Layout::from_size_align(total_size, 8).expect("Checked");
        let raw = unsafe { alloc(layout) };
        if raw.is_null() {
            handle_alloc_error(layout);
        }
        let ptr = unsafe { raw.add(size_of::<GcHeader>()) };
        unsafe {
            let header = GcHeader::from_obj_ptr(ptr as *const T);
            header.init::<T>();

            (ptr as *mut T).write(val);
        }
        self.larges
            .push(LargeObject(raw, layout, TypeId::of::<T>(), drop_box::<T>));
        self.total_objects += 1;

        Gc {
            ptr: unsafe { NonNull::new_unchecked(ptr) },
            _marker: PhantomData,
        }
    }

    pub fn alloc<T: Trace + 'static>(&mut self, val: T) -> Gc<T> {
        let obj_size = size_of::<T>();

        if let Some(class) = size_class_index(obj_size) {
            let page = self.find_page_for_class(class);
            let obj_ptr = page.alloc().expect("Checked in find_page_for_class");
            unsafe {
                // 创建GC头
                let header = GcHeader::from_obj_ptr(obj_ptr);
                header.init::<T>();
                // 写入对象实际数据
                (obj_ptr as *mut T).write(val);
            }

            self.total_objects += 1;

            Gc {
                ptr: unsafe { NonNull::new_unchecked(obj_ptr) },
                _marker: PhantomData,
            }
        } else {
            self.alloc_large(val)
        }
    }

    #[inline]
    /// Collect garbage without finalizers
    pub fn collect(&mut self, roots: &[&dyn Trace]) {
        self.collect_with_finalizer(roots, |_, _| {});
    }

    /// Collect garbage and run finalizer before drop a object
    pub fn collect_with_finalizer<F>(&mut self, roots: &[&dyn Trace], mut finalizer: F)
    where
        F: FnMut(*const (), TypeId),
    {
        let mut tracer = Tracer { heap: self };

        for root in roots {
            root.trace(&mut tracer);
        }

        let before_count = self.total_objects;
        for pages in &mut self.page_map {
            for page in pages.iter_mut() {
                for ptr in page
                    .iter_allocated()
                    .filter_map(|(_, ptr)| unsafe {
                        let header = GcHeader::from_obj_ptr(ptr);
                        (header.get_color() == GcColor::White).then_some(ptr as *const ())
                    })
                    .collect::<Vec<_>>()
                {
                    unsafe {
                        let obj_ptr = ptr as *mut u8;
                        let header = page.header_of(obj_ptr);
                        finalizer(ptr, header.type_id);
                        (header.destructor)(obj_ptr);
                        page.dealloc(obj_ptr);
                    }
                    self.total_objects -= 1;
                }
            }
        }

        self.larges
            .retain(|LargeObject(raw, layout, type_id, destructor)| {
                let obj_ptr = unsafe { raw.add(size_of::<GcHeader>()) };
                let ptr = obj_ptr as *const ();
                if unsafe { GcHeader::from_obj_ptr(obj_ptr).get_color() == GcColor::White } {
                    finalizer(ptr, *type_id);
                    unsafe {
                        destructor(obj_ptr);
                        dealloc(*raw, *layout);
                    }
                    self.total_objects -= 1;
                    false
                } else {
                    true
                }
            });

        let after_count = self.total_objects;
        let freed = before_count.saturating_sub(after_count);
        self.next_gc = if freed > 0 {
            (self.total_objects * 2).max(self.threshold)
        } else {
            self.total_objects + self.threshold
        };
    }

    #[inline]
    pub fn ptr_for<T>(&self, gc: &Gc<T>) -> *const () {
        gc.as_ptr()
    }
}
