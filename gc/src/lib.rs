//! GC
use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::any::TypeId;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::header::GcColor;
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
struct LargeObject {
    raw: *mut u8,
    obj: *mut u8,
    layout: Layout,
    type_id: TypeId,
    destructor: unsafe fn(*mut u8),
}

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

impl Drop for Heap {
    fn drop(&mut self) {
        for pages in &self.page_map {
            for page in pages {
                for (_, obj_ptr) in page.iter_allocated() {
                    unsafe {
                        (page.header_of(obj_ptr).destructor)(obj_ptr);
                    }
                }
            }
        }

        for object in self.larges.drain(..) {
            unsafe {
                (object.destructor)(object.obj);
                dealloc(object.raw, object.layout);
            }
        }
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

    /// The hook the allocator is expected to check, `total_objects >= next_gc`
    /// `set_threshold` resets `next_gc` right away so the next collection waits
    /// for that many fresh allocations again
    ///
    /// ```
    /// use duka_gc::{Heap, Trace};
    ///
    /// struct Blob(u32);
    /// impl Trace for Blob {}
    ///
    /// let mut heap = Heap::new();
    /// assert!(!heap.should_collect());
    ///
    /// heap.set_threshold(2);
    /// heap.alloc(Blob(1));
    /// assert!(!heap.should_collect());
    /// heap.alloc(Blob(2));
    /// assert!(heap.should_collect());
    /// ```
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
        let alignment = std::mem::align_of::<T>().max(std::mem::align_of::<GcHeader>());
        let total_size = GcHeader::total_size::<T>()
            .checked_add(alignment - 1)
            .expect("GC allocation size overflow");
        let layout = Layout::from_size_align(total_size, alignment).expect("Checked");
        let raw = unsafe { alloc(layout) };
        if raw.is_null() {
            handle_alloc_error(layout);
        }
        let header_size = size_of::<GcHeader>();
        let object_start = unsafe { raw.add(header_size) };
        let padding = object_start.align_offset(alignment);
        let ptr = unsafe { object_start.add(padding) };
        unsafe {
            GcHeader::init_for(ptr as *mut T);
            (ptr as *mut T).write(val);
        }
        self.larges.push(LargeObject {
            raw,
            obj: ptr,
            layout,
            type_id: TypeId::of::<T>(),
            destructor: crate::header::drop_box::<T>,
        });
        self.total_objects += 1;

        Gc {
            ptr: unsafe { NonNull::new_unchecked(ptr) },
            _marker: PhantomData,
        }
    }

    pub fn alloc<T: Trace + 'static>(&mut self, val: T) -> Gc<T> {
        let obj_size = size_of::<T>();

        if std::mem::align_of::<T>() <= std::mem::align_of::<GcHeader>()
            && let Some(class) = size_class_index(obj_size)
        {
            let page = self.find_page_for_class(class);
            let obj_ptr = page.alloc().expect("Checked in find_page_for_class");
            unsafe {
                GcHeader::init_for(obj_ptr as *mut T);
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
        self.collect_with_finalizer_roots(
            roots,
            |_, _, _| {},
            |ptr, type_id| {
                finalizer(ptr, type_id);
            },
        );
    }

    /// `prepare_finalizer` can mark pending finalizer values before sweep; `finalizer`
    /// runs only for objects that remain unreachable after those values are traced.
    pub fn collect_with_finalizer_roots<P, F>(
        &mut self,
        roots: &[&dyn Trace],
        mut prepare_finalizer: P,
        mut finalizer: F,
    ) where
        P: FnMut(*const (), TypeId, &mut Tracer),
        F: FnMut(*const (), TypeId),
    {
        self.gray_list.clear();
        for pages in &self.page_map {
            for page in pages {
                for (header, _) in page.iter_allocated() {
                    unsafe { (*header).set_color(GcColor::White) };
                }
            }
        }
        for object in &self.larges {
            unsafe { GcHeader::from_obj_ptr(object.obj).set_color(GcColor::White) };
        }

        let mut tracer = Tracer { heap: self };
        for root in roots {
            root.trace(&mut tracer);
        }

        while let Some(obj_ptr) = tracer.heap.gray_list.pop() {
            let trace_fn = unsafe { GcHeader::from_obj_ptr(obj_ptr).trace_fn };
            unsafe {
                trace_fn(obj_ptr, &mut tracer);
                GcHeader::from_obj_ptr(obj_ptr).set_color(GcColor::Black);
            }
        }

        drop(tracer);

        let mut prepare_targets = vec![];
        for pages in &self.page_map {
            for page in pages {
                for (_, obj_ptr) in page.iter_allocated() {
                    let header = unsafe { GcHeader::from_obj_ptr(obj_ptr) };
                    if header.get_color() == GcColor::White {
                        prepare_targets.push((obj_ptr, header.type_id));
                    }
                }
            }
        }
        for object in &self.larges {
            let header = unsafe { GcHeader::from_obj_ptr(object.obj) };
            if header.get_color() == GcColor::White {
                prepare_targets.push((object.obj, object.type_id));
            }
        }

        let mut tracer = Tracer { heap: self };
        for (obj_ptr, type_id) in prepare_targets {
            if unsafe { GcHeader::from_obj_ptr(obj_ptr).get_color() } == GcColor::White {
                prepare_finalizer(obj_ptr as *const (), type_id, &mut tracer);
            }
        }

        while let Some(obj_ptr) = tracer.heap.gray_list.pop() {
            let trace_fn = unsafe { GcHeader::from_obj_ptr(obj_ptr).trace_fn };
            unsafe {
                trace_fn(obj_ptr, &mut tracer);
                GcHeader::from_obj_ptr(obj_ptr).set_color(GcColor::Black);
            }
        }

        drop(tracer);

        let before_count = self.total_objects;
        for pages in &mut self.page_map {
            for page in pages.iter_mut() {
                let dead = page
                    .iter_allocated()
                    .filter_map(|(_, ptr)| unsafe {
                        let header = GcHeader::from_obj_ptr(ptr);
                        if header.get_color() == GcColor::White {
                            Some(ptr)
                        } else {
                            header.set_color(GcColor::White);
                            None
                        }
                    })
                    .collect::<Vec<_>>();

                for obj_ptr in dead {
                    let destructor;
                    unsafe {
                        let header = page.header_of(obj_ptr);
                        let type_id = header.type_id;
                        destructor = header.destructor;
                        finalizer(obj_ptr as *const (), type_id);
                        page.dealloc(obj_ptr);
                    }
                    self.total_objects -= 1;
                    unsafe { destructor(obj_ptr) };
                }
            }
        }

        let mut index = 0;
        while index < self.larges.len() {
            let (obj, raw, layout, type_id, destructor, color) = {
                let object = &self.larges[index];
                let color = unsafe { GcHeader::from_obj_ptr(object.obj).get_color() };
                (
                    object.obj,
                    object.raw,
                    object.layout,
                    object.type_id,
                    object.destructor,
                    color,
                )
            };
            if color == GcColor::White {
                finalizer(obj as *const (), type_id);
                self.larges.swap_remove(index);
                self.total_objects -= 1;
                unsafe {
                    destructor(obj);
                    dealloc(raw, layout);
                }
            } else {
                unsafe { GcHeader::from_obj_ptr(obj).set_color(GcColor::White) };
                index += 1;
            }
        }

        let after_count = self.total_objects;
        let freed = before_count.saturating_sub(after_count);
        self.next_gc = if freed > 0 {
            self.total_objects.saturating_mul(2).max(self.threshold)
        } else {
            self.total_objects.saturating_add(self.threshold)
        };
    }

    #[inline]
    pub fn ptr_for<T>(&self, gc: &Gc<T>) -> *const () {
        gc.as_ptr()
    }
}

#[cfg(test)]
mod tests {
    use super::{Gc, Heap, Trace, Tracer};
    use std::any::TypeId;
    use std::cell::Cell;
    use std::rc::Rc;

    struct Node {
        child: Option<Gc<Node>>,
    }

    impl Trace for Node {
        fn trace(&self, tracer: &mut Tracer) {
            if let Some(child) = &self.child {
                tracer.mark(child);
            }
        }
    }

    struct Root(Gc<Node>);

    impl Trace for Root {
        fn trace(&self, tracer: &mut Tracer) {
            tracer.mark(&self.0);
        }
    }

    #[test]
    fn collection_traces_all_reachable_levels_and_resets_colors() {
        let mut heap = Heap::new();
        let leaf = heap.alloc(Node { child: None });
        let middle = heap.alloc(Node { child: Some(leaf) });
        let root_node = heap.alloc(Node {
            child: Some(middle),
        });
        let _garbage = heap.alloc(Node { child: None });
        let root = Root(root_node);

        heap.collect(&[&root]);
        assert_eq!(heap.allocation_count(), 3);

        heap.collect(&[&root]);
        assert_eq!(heap.allocation_count(), 3);

        heap.collect(&[]);
        assert_eq!(heap.allocation_count(), 0);
    }

    struct Finalizable {
        finalizer_capture: Gc<Node>,
    }

    impl Trace for Finalizable {
        fn trace(&self, tracer: &mut Tracer) {
            tracer.mark(&self.finalizer_capture);
        }
    }

    #[test]
    fn finalizer_roots_are_traced_before_sweep() {
        let mut heap = Heap::new();
        let capture = heap.alloc(Node { child: None });
        let finalizable = heap.alloc(Finalizable {
            finalizer_capture: capture,
        });
        let garbage = heap.alloc(Node { child: None });
        let mut swept = vec![];

        heap.collect_with_finalizer_roots(
            &[],
            |ptr, type_id, tracer| {
                if type_id == TypeId::of::<Finalizable>() {
                    let object = unsafe { &*(ptr as *const Finalizable) };
                    tracer.mark(&object.finalizer_capture);
                }
            },
            |ptr, _| swept.push(ptr as usize),
        );

        assert_eq!(heap.allocation_count(), 1);
        assert!(swept.contains(&(finalizable.as_ptr() as usize)));
        assert!(swept.contains(&(garbage.as_ptr() as usize)));
        assert!(!swept.contains(&(capture.as_ptr() as usize)));
    }

    #[repr(align(64))]
    struct AlignedLarge {
        dropped: Option<Rc<Cell<usize>>>,
        _data: [u8; 600],
    }

    impl Trace for AlignedLarge {}

    impl Drop for AlignedLarge {
        fn drop(&mut self) {
            if let Some(dropped) = &self.dropped {
                dropped.set(dropped.get() + 1);
            }
        }
    }

    impl<T: Trace> Trace for GcRoot<T> {
        fn trace(&self, tracer: &mut Tracer) {
            tracer.mark(&self.0);
        }
    }

    struct GcRoot<T: Trace>(Gc<T>);

    #[test]
    fn large_objects_preserve_alignment_and_finalize_once() {
        let mut heap = Heap::new();
        let dropped = Rc::new(Cell::new(0));
        let object = heap.alloc(AlignedLarge {
            dropped: Some(dropped.clone()),
            _data: [0; 600],
        });
        let root = GcRoot(object);

        heap.collect(&[&root]);
        assert_eq!(
            object.as_ptr() as usize % std::mem::align_of::<AlignedLarge>(),
            0
        );
        assert_eq!(heap.allocation_count(), 1);

        let mut finalized = 0;
        heap.collect_with_finalizer(&[], |ptr, type_id| {
            assert_eq!(ptr, object.as_ptr());
            assert_eq!(type_id, std::any::TypeId::of::<AlignedLarge>());
            finalized += 1;
        });

        assert_eq!(finalized, 1);
        assert_eq!(heap.allocation_count(), 0);
        assert_eq!(dropped.get(), 1);
    }

    struct DropProbe(Rc<Cell<usize>>);

    impl Trace for DropProbe {}

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn heap_drop_drops_allocated_objects() {
        let dropped = Rc::new(Cell::new(0));
        {
            let mut heap = Heap::new();
            heap.alloc(DropProbe(dropped.clone()));
            heap.alloc(AlignedLarge {
                dropped: Some(dropped.clone()),
                _data: [0; 600],
            });
        }

        assert_eq!(dropped.get(), 2);
    }
}
