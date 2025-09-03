use std::{
    any::Any,
    cell::RefCell,
    collections::{HashSet, VecDeque},
    marker::PhantomData,
};

pub trait GcObj: Any + Send + Sync {
    /// 此对象引用的子对象
    fn references(&self) -> Vec<GcPtr>;
    fn size(&self) -> usize;

    fn as_any(&self) -> &dyn Any;
    fn as_mut_any(&mut self) -> &mut dyn Any;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GcPtr {
    index: usize,
    generation: u8,
}
impl GcPtr {
    fn from_index(index: usize) -> Self {
        Self {
            index,
            generation: 0,
        }
    }
}

// use lifetime param to simulate context
pub struct GcContext<'a>(PhantomData<&'a ()>);
impl GcContext<'_> {
    pub fn allocate<T: GcObj>(&self, obj: T) -> GcPtr {
        crate::core::global_alloc(obj)
    }
    pub fn add_root(&self, ptr: GcPtr) {
        crate::core::add_temp_root(ptr);
    }
}

/// in heap
struct GcSlot {
    inner: Option<Box<dyn GcObj>>,
    marked: bool,
    size: usize,
}
impl GcSlot {
    fn empty() -> Self {
        Self {
            inner: None,
            marked: false,
            size: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GcConfig {
    pub init_heap_size: usize,
    pub collect_threshold: f32,
    pub growth_factor: f32,
    pub enable_generations: bool,
    pub enable_incremental: bool,
}

struct GarbageCollector {
    heap: Vec<GcSlot>,
    roots: HashSet<GcPtr>,
    temp_roots: HashSet<GcPtr>,
    free_list: Vec<usize>,
    allocated: usize,
    threshold: usize,
    config: GcConfig,
}
impl GarbageCollector {
    fn new(config: GcConfig) -> Self {
        let hs = config.init_heap_size;
        let mut heap = Vec::with_capacity(hs);
        let mut free_list = Vec::with_capacity(hs);

        for i in 0..hs {
            heap.push(GcSlot::empty());
            free_list.push(i);
        }

        Self {
            roots: HashSet::new(),
            temp_roots: HashSet::new(),
            allocated: 0,
            threshold: (hs as f32 * config.collect_threshold) as usize,

            heap,
            free_list,
            config,
        }
    }

    fn add_root(&mut self, root: GcPtr) {
        self.roots.insert(root);
    }
    fn remove_root(&mut self, root: &GcPtr) {
        self.roots.remove(root);
    }
    fn add_temp_root(&mut self, temp_root: GcPtr) {
        self.temp_roots.insert(temp_root);
    }
    fn clear_temp_root(&mut self) {
        self.temp_roots.clear();
    }

    fn allocate<T: GcObj>(&mut self, obj: T) -> GcPtr {
        // collect first
        let size = obj.size();

        if self.allocated + size > self.threshold {
            self.collect_garbage();
        }

        let index = self.free_list.pop().unwrap_or_else(|| {
            self.expand_heap();
            self.free_list.pop().expect("Failed to expand heap")
        });
        self.allocated += size;

        GcPtr::from_index(index)
    }

    fn get<T: GcObj>(&self, ptr: GcPtr) -> Option<&T> {
        self.heap
            .get(ptr.index)
            .and_then(|s| s.inner.as_ref())
            .and_then(|o| o.as_any().downcast_ref::<T>())
    }
    fn get_mut<T: GcObj>(&mut self, ptr: GcPtr) -> Option<&mut T> {
        self.heap
            .get_mut(ptr.index)
            .and_then(|s| s.inner.as_mut())
            .and_then(|o| o.as_mut_any().downcast_mut::<T>())
    }

    fn collect_garbage(&mut self) {
        self.mark();
        self.sweep();
        self.threshold = (self.allocated as f32 * self.config.growth_factor) as usize;
    }

    /* #region mark & sweep */
    fn mark(&mut self) {
        let mut worklist = VecDeque::new();

        // marking from roots
        let roots = self.roots.union(&self.temp_roots);
        for root in roots {
            let Some(slot) = self.heap.get_mut(root.index) else {
                continue;
            };
            if !slot.marked {
                slot.marked = true;
                worklist.push_back(*root);
            }
        }

        // marking children
        while let Some(ptr) = worklist.pop_front() {
            let Some(slot) = self.heap.get_mut(ptr.index) else {
                continue;
            };
            let Some(obj) = &slot.inner else { continue };
            for child in obj.references() {
                let Some(slot) = self.heap.get_mut(child.index) else {
                    continue;
                };
                if !slot.marked {
                    slot.marked = true;
                    worklist.push_back(child);
                }
            }
        }
    }

    fn sweep(&mut self) {
        for (i, slot) in self.heap.iter_mut().enumerate() {
            if *&slot.inner.is_none() {
                continue;
            }

            // 在mark中已标记了一轮
            if slot.marked {
                slot.marked = false;
            } else {
                // 释放不可触及的对象
                self.free_list.push(i);
                self.allocated -= slot.size;

                *slot = GcSlot::empty();
            }
        }
    }

    fn expand_heap(&mut self) {
        let size = self.heap.capacity();
        let new_size = (size as f32 * self.config.growth_factor) as usize;

        self.heap.reserve(new_size - size);
        for i in size..new_size {
            self.heap.push(GcSlot::empty());
            self.free_list.push(i);
        }
    }

    /* #endregion mark & sweep */
}

const INIT_SIZE: usize = 1024 * 1024;
const THRESHOLD: f32 = 1.5;
const FACTOR: f32 = 0.7;
thread_local! {
    static GLOBAL_GC: RefCell<GarbageCollector> = RefCell::new(
        GarbageCollector::new(GcConfig {
            init_heap_size: INIT_SIZE,
            collect_threshold: THRESHOLD,
            growth_factor: FACTOR,
            enable_generations: true,
            enable_incremental: true
        })
    );
}

pub fn global_alloc<T: GcObj>(obj: T) -> GcPtr {
    GLOBAL_GC.with_borrow_mut(|gc| gc.allocate(obj))
}
pub fn add_root(ptr: GcPtr) {
    GLOBAL_GC.with_borrow_mut(|gc| gc.add_root(ptr));
}
pub fn remove_root(ptr: &GcPtr) {
    GLOBAL_GC.with_borrow_mut(|gc| gc.remove_root(ptr));
}
pub fn add_temp_root(ptr: GcPtr) {
    GLOBAL_GC.with_borrow_mut(|gc| gc.add_temp_root(ptr));
}
pub fn clear_temp_root() {
    GLOBAL_GC.with_borrow_mut(|gc| gc.clear_temp_root());
}

pub fn collect_garbage() {
    GLOBAL_GC.with_borrow_mut(|gc| gc.collect_garbage());
}
pub fn with_context<F, R>(f: F) -> R
where
    F: for<'a> FnOnce(GcContext<'a>) -> R,
{
    let res = f(GcContext(PhantomData));
    clear_temp_root();
    res
}
