use std::{cell::Cell, marker::PhantomData, ops::Deref};

/// 所有GC对象必须实现 用于递归标记可达对象
pub trait Trace {
    fn trace(&self, tracer: &mut dyn FnMut(&GcObject));
}

/// 堆上的数据 带有标记位
pub struct GcObject {
    pub marked: Cell<bool>,
    pub value: Box<dyn Trace>,
}

/// Gc智能指针
#[derive(Debug)]
pub struct Gc<T: Trace + 'static> {
    /// raw pointer
    ptr: *mut GcObject,
    _marker: PhantomData<T>,
}
impl<T: Trace + 'static> Gc<T> {
    pub fn as_gc_object(&self) -> &GcObject {
        unsafe { &*self.ptr }
    }
}
impl<T: Trace + 'static> Clone for Gc<T> {
    fn clone(&self) -> Self {
        Gc {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}
impl<T: Trace + 'static> Deref for Gc<T> {
    type Target = T;

    /// # UNSAFE
    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.ptr as *const T) }
    }
}
impl<T: Trace + 'static> Drop for Gc<T> {
    fn drop(&mut self) {
        // nothing, 让GC回收
    }
}

#[derive(Debug)]
pub enum GcState {
    Idle,
    Marking,
    Sweeping,
}

/// 管理器 统一对象的注册并回收
/// ---
/// - 从roots出发 递归trace 设置`marked = true`
/// - 遍历*objects* 释放`marked == false`的对象
#[derive(Debug)]
pub struct GcHeap {
    objects: Vec<*mut GcObject>,
    mark_queue: Vec<*const GcObject>,
    sweep_index: usize,
    state: GcState,

    /// 总分配次数
    pub alloc_count: usize,
    /// 总释放次数
    pub free_count: usize,
}
impl GcHeap {
    pub fn new() -> Self {
        Self {
            objects: vec![],
            mark_queue: vec![],
            sweep_index: 0,
            state: GcState::Idle,

            alloc_count: 0,
            free_count: 0,
        }
    }

    pub fn step(&mut self) {
        match self.state {
            GcState::Idle => {}
            GcState::Marking => {
                if !self.mark_queue.is_empty() {
                    let obj = self.mark_queue.pop().unwrap();
                    let gc_box = unsafe { &*obj };
                    if !gc_box.marked.get() {
                        gc_box.marked.set(true);
                        gc_box
                            .value
                            .trace(&mut |child| self.mark_queue.push(child as *const _));
                    }
                } else {
                    self.state = GcState::Sweeping;
                }
            }
            GcState::Sweeping => {
                if self.sweep_index < self.objects.len() {
                    let ptr = self.objects[self.sweep_index];
                    let gc_box = unsafe { &*ptr };
                    if !gc_box.marked.get() {
                        self.free_count += 1;
                        unsafe {
                            let _ = Box::from_raw(ptr);
                        }
                        self.objects.remove(self.sweep_index);
                    } else {
                        gc_box.marked.set(false);
                        self.sweep_index += 1;
                    }
                } else {
                    // 结束 重置
                    self.state = GcState::Idle;
                    self.sweep_index = 0;
                }
            }
        }
    }

    /// 分配
    pub fn allocate<T: Trace + 'static>(&mut self, val: T) -> Gc<T> {
        self.alloc_count += 1;

        let obj = Box::new(GcObject {
            marked: Cell::new(false),
            value: Box::new(val),
        });
        let ptr = Box::into_raw(obj);
        self.objects.push(ptr);

        dbg!(&self.objects);

        Gc {
            ptr,
            _marker: PhantomData,
        }
    }

    /// 标记所有可达对象 清理无标记对象
    pub fn start_gc(&mut self, roots: &[&GcObject]) {
        self.mark_queue.clear();
        for root in roots {
            self.mark_queue.push(*root as *const _);
        }
        self.state = GcState::Marking;
        self.sweep_index = 0;
    }
}
