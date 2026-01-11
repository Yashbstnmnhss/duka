//! GC
use std::cell::UnsafeCell;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::ptr::NonNull;

pub mod prelude {
    pub use super::{Finalize, Gc, GcCell, GcCellRef, GcCellRefMut, Heap, Trace, Tracer};
}

pub trait Trace {
    fn trace(&self, _tracer: &mut Tracer) {}
}
pub trait Finalize {
    fn finalize(&self) {}
}
pub struct Tracer<'a> {
    pub heap: &'a mut Heap,
    pub marked: &'a mut HashSet<*const ()>,
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

    pub fn new(value: T) -> Self {
        let bx = Box::new(value);
        let ptr = Box::into_raw(bx) as *mut u8;
        let nn = unsafe { NonNull::new_unchecked(ptr) };
        Gc {
            ptr: nn,
            _marker: PhantomData,
        }
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
    pub unsafe fn get(&self) -> &T {
        unsafe { &*self.inner.get() }
    }
    pub unsafe fn get_mut(&self) -> &mut T {
        unsafe { &mut *self.inner.get() }
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
    destructor: unsafe fn(*mut u8),
}

#[derive(Debug)]
pub struct Heap {
    allocations: Vec<Allocation>,
}

impl Heap {
    pub fn new() -> Self {
        Heap {
            allocations: Vec::new(),
        }
    }

    pub fn alloc<T: Trace + 'static>(&mut self, _value: T) -> Gc<T> {
        let bx = Box::new(_value);
        let ptr = Box::into_raw(bx) as *mut u8;
        let nn = unsafe { NonNull::new_unchecked(ptr) };

        unsafe fn drop_box<T>(p: *mut u8) {
            let tptr = p as *mut T;
            unsafe { drop(Box::from_raw(tptr)) };
        }

        let destructor: unsafe fn(*mut u8) = drop_box::<T>;
        self.allocations.push(Allocation { ptr, destructor });

        Gc {
            ptr: nn,
            _marker: PhantomData,
        }
    }

    pub fn collect(&mut self, roots: &[&dyn Trace]) {
        let mut marked: HashSet<*const ()> = HashSet::new();
        let mut tracer = Tracer {
            heap: self,
            marked: &mut marked,
        };

        for root in roots {
            root.trace(&mut tracer);
        }

        self.allocations.retain(|alloc| {
            let p = alloc.ptr as *const ();
            if marked.contains(&p) {
                true
            } else {
                unsafe { (alloc.destructor)(alloc.ptr) };
                false
            }
        });
    }

    pub fn ptr_for<T>(&self, gc: &Gc<T>) -> *const () {
        gc.as_ptr()
    }
}
