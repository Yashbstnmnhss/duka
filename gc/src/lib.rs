//! GC
use std::any::TypeId;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ptr::NonNull;

use hashbrown::HashSet;
use rustc_hash::FxBuildHasher;

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
    pub marked: &'a mut HashSet<*const (), FxBuildHasher>,
}
impl<'a> Tracer<'a> {
    pub fn mark<T: Trace>(&mut self, gc: &Gc<T>) {
        let p = gc.ptr.as_ptr() as *const ();
        if !self.marked.insert(p) {
            return;
        }

        let obj = gc.ptr.as_ptr() as *const T;
        let r = unsafe { &*obj };
        r.trace(self);
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

    // /// - `*mut u8`: 指向u8(一字节)数据类型的**指针**
    // /// - `Box::into_raw`
    // pub(crate) fn from_raw(value: T) -> Self {
    //     let bx = Box::new(value);
    //     let ptr = Box::into_raw(bx) as *mut u8;
    //     let nn = unsafe { NonNull::new_unchecked(ptr) };
    //     Gc {
    //         ptr: nn,
    //         _marker: PhantomData,
    //     }
    // }
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

    // # Safety
    // 不得存在其他可变引用!
    // 使用 `borrow()` 替代
    // unsafe fn get(&self) -> &T {
    //     unsafe { &*self.inner.get() }
    // }

    // # Safety
    // 不得存在其他任何引用!
    // 使用 `borrow_mut()` 替代
    // unsafe fn get_mut(&mut self) -> &mut T {
    //     unsafe { &mut *self.inner.get() }
    // }

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
struct Allocation {
    ptr: *mut u8,
    type_id: TypeId,
    destructor: unsafe fn(*mut u8),
}

#[derive(Debug)]
pub struct Heap {
    allocations: Vec<Allocation>,
    /// GC 触发阈值：当分配数超过此值时触发 GC
    threshold: usize,
    /// 下次 GC 的阈值（动态调整）
    next_gc: usize,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    pub fn new() -> Self {
        Heap {
            allocations: vec![],
            threshold: 256, // 默认阈值
            next_gc: 256,
        }
    }

    /// 检查是否需要触发 GC
    pub fn should_collect(&self) -> bool {
        self.allocations.len() >= self.next_gc
    }

    /// 设置 GC 触发阈值
    pub fn set_threshold(&mut self, threshold: usize) {
        self.threshold = threshold;
        self.next_gc = threshold;
    }

    /// 获取当前分配数
    pub fn allocation_count(&self) -> usize {
        self.allocations.len()
    }

    pub fn alloc<T: Trace + 'static>(&mut self, val: T) -> Gc<T> {
        let bx = Box::new(val);
        let ptr = Box::into_raw(bx) as *mut u8;
        let nn = unsafe { NonNull::new_unchecked(ptr) };

        /// # SAFETY
        /// maybe, I think it is safe
        unsafe fn drop_box<T>(p: *mut u8) {
            let tptr = p as *mut T;
            unsafe { drop(Box::from_raw(tptr)) };
        }

        let destructor: unsafe fn(*mut u8) = drop_box::<T>;
        self.allocations.push(Allocation {
            ptr,
            type_id: TypeId::of::<T>(),
            destructor,
        });

        Gc {
            ptr: nn,
            _marker: PhantomData,
        }
    }

    /// 执行GC
    pub fn collect(&mut self, roots: &[&dyn Trace]) {
        self.collect_with_finalizer(roots, |_, _| {});
    }

    /// 执行GC，并在销毁对象前调用 finalizer
    ///
    /// 每个被回收对象的指针和类型 ID 都会传给 finalizer，因为堆里的分配是
    /// 异构的（`HeapString`、`GcCell<..>`、`DukaClosure` 等），调用方必须用
    /// `type_id` 判断类型后才能安全解引用 `ptr`
    pub fn collect_with_finalizer<F>(&mut self, roots: &[&dyn Trace], mut finalizer: F)
    where
        F: FnMut(*const (), TypeId),
    {
        let mut marked: HashSet<*const (), FxBuildHasher> =
            HashSet::with_capacity_and_hasher(0, FxBuildHasher);
        let mut tracer = Tracer {
            heap: self,
            marked: &mut marked,
        };

        for root in roots {
            root.trace(&mut tracer);
        }

        let before_count = self.allocations.len();

        self.allocations.retain(|alloc| {
            let p = alloc.ptr as *const ();
            if marked.contains(&p) {
                true
            } else {
                // 调用 finalizer
                finalizer(p, alloc.type_id);
                // 然后销毁对象
                unsafe { (alloc.destructor)(alloc.ptr) };
                false
            }
        });

        // 动态调整下次 GC
        let after_count = self.allocations.len();
        let freed = before_count.saturating_sub(after_count);
        if freed > 0 {
            // 如果释放了很多对象 增加阈值
            self.next_gc = (self.allocations.len() * 2).max(self.threshold);
        } else {
            // 如果没有释放对象 保持当前阈值
            self.next_gc = self.allocations.len() + self.threshold;
        }
    }

    pub fn ptr_for<T>(&self, gc: &Gc<T>) -> *const () {
        gc.as_ptr()
    }
}
