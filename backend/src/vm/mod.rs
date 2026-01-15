use std::collections::HashMap;

use crate::{
    DukaVM,
    error::DukaRuntimeError,
    instructions::{Address, Bits25},
    value::{DukaClosure, DukaProto, RuntimeValue, RustClosure, ValueCount},
    vm::{
        coroutine::{CoState, Coroutine, CoroutineID, CoroutineStatus},
        frame::CallFrame,
    },
};
use duka_shared::constants::{MetaMethod, ctype, sugar};
use gc::prelude::*;
use gc::{Finalize, Trace, Tracer};

pub mod coroutine;
pub mod frame;

#[derive(Debug)]
/// Action of a running coroutine
pub enum CoAction {
    /// Return values, coroutine dead
    Return(Address, ValueCount),
    /// Yield and suspend coroutine
    Yield(Address, ValueCount),
    /// Call another coroutine
    Go(CoroutineID, Address, ValueCount),

    // Create a new coroutine
    Spawn(Address, Address),
}

/// # 协程调度器
/// 管理所有协程及其创建、切换、销毁
#[derive(Debug)]
pub struct Scheduler {
    coroutines: HashMap<CoroutineID, Gc<GcCell<Coroutine>>>, // ID to coroutine
    current: CoroutineID,                                    // current running coroutine
    free_list: Vec<CoroutineID>,                             // IDs of previous dead coroutines
    id_sp: CoroutineID,                                      // the newest ID, not be used yet
}
impl Scheduler {
    /// ID of the main coroutine
    pub const MAIN_ID: CoroutineID = 0;

    fn gen_id(&mut self) -> CoroutineID {
        self.free_list.pop().unwrap_or_else(|| {
            let id = self.id_sp;
            self.id_sp += 1;
            id
        })
    }

    /// ### This will create a initial coroutine *(main coroutine)* with `id = MAIN_ID`
    pub fn with_main(main: CoState, heap: &mut Heap) -> Self {
        let mut coroutines = HashMap::new();
        coroutines.insert(
            Self::MAIN_ID,
            heap.alloc(GcCell::new(Coroutine::new(Self::MAIN_ID, main, None))),
        );

        Self {
            free_list: vec![],
            id_sp: Self::MAIN_ID,
            current: 0,
            coroutines,
        }
    }

    /// Create a coroutine and switch to it, returning its ID
    #[inline]
    pub fn create_switch(&mut self, state: CoState, heap: &mut Heap) -> CoroutineID {
        let id = self.create(state, heap);
        self.switch(id);
        id
    }

    /// ### Create a coroutine with its CoState, returning its ID
    pub fn create(&mut self, state: CoState, heap: &mut Heap) -> CoroutineID {
        let id = self.gen_id();
        let cor = Coroutine {
            inner: state,
            status: CoroutineStatus::Ready,
            id,
            parent: Some(self.current),

            last_wanted: 0,
        };
        // allocate coroutine cell on provided heap
        self.coroutines.insert(id, heap.alloc(GcCell::new(cor)));
        id
    }

    pub fn destroy(&mut self, id: CoroutineID) {
        if self.coroutines.remove(&id).is_some() {
            self.free_list.push(id);
        }
    }

    /// ### main loop
    pub fn go(
        &mut self,
        ctx: &mut VMContext,
        heap: &mut gc::Heap,
    ) -> Result<ValueCount, DukaRuntimeError> {
        use CoAction::*;
        Ok(loop {
            let result = self.current_mut().execute(ctx, heap)?;
            match result {
                Return(from, return_count) => {
                    if self.is_main() {
                        self.main_mut().status = CoroutineStatus::Dead;
                        break return_count.into();
                    }
                    let id = self.current;

                    let results = self
                        .current_mut()
                        .inner
                        .cut_stack(from as usize, return_count);

                    self.switch_parent();
                    self.current_mut().inner.stack.extend(results);

                    self.destroy(id);
                }
                Yield(from, yield_count) => {
                    if self.is_main() {
                        self.main_mut().status = CoroutineStatus::Suspended;
                        break yield_count.into();
                    }

                    let yieldeds = self
                        .current_mut()
                        .inner
                        .cut_stack(from as usize, yield_count);

                    self.switch_parent();
                    self.current_mut().inner.stack.extend(yieldeds);
                }
                Go(to, from, params_count) => {
                    let mut params = self
                        .current_mut()
                        .inner
                        .cut_stack(from as usize, params_count);
                    self.switch(to);

                    let wanted = self.current().last_wanted;
                    if params.len() > wanted {
                        params.drain(wanted..);
                    } else {
                        for _ in 0..wanted - params.len() {
                            params.push(RuntimeValue::Nil);
                        }
                    }

                    self.current_mut().inner.stack.extend(params);
                }
                Spawn(ad, from) => {
                    let closure = match self.current().inner.get_stack(from as usize)? {
                        RuntimeValue::UserFunc(c) => c.clone(),
                        _ => return Err(DukaRuntimeError::InvalidValueType(ctype::CLO)),
                    };
                    let id = self.create(CoState::from_closure(closure), heap);
                    self.current_mut()
                        .inner
                        .set_stack(ad as usize, RuntimeValue::Coroutine(id))?;
                }
            }
        })
    }

    #[inline]
    pub fn switch(&mut self, to: CoroutineID) {
        if self.current == to {
            return;
        }
        if to < self.coroutines.len() && !self.free_list.contains(&to) {
            self.current_mut().status = CoroutineStatus::Suspended;
            self.current = to;
            self.current_mut().status = CoroutineStatus::Running;
        }
    }

    #[inline(always)]
    const fn is_main(&self) -> bool {
        self.current == Self::MAIN_ID
    }

    /// ### switch to the main coroutine
    #[inline(always)]
    pub fn switch_parent(&mut self) {
        let parent = self.current().parent;
        self.switch(parent.unwrap_or(Self::MAIN_ID));
    }

    #[inline(always)]
    pub fn main(&self) -> GcCellRef<'_, Coroutine> {
        self.coroutines
            .get(&Self::MAIN_ID)
            .expect("NO MAIN COROUTINE")
            .borrow()
    }
    #[inline(always)]
    pub fn main_mut(&self) -> GcCellRefMut<'_, Coroutine> {
        self.coroutines
            .get(&Self::MAIN_ID)
            .expect("NO MAIN COROUTINE")
            .borrow_mut()
    }

    #[inline(always)]
    pub fn current(&self) -> GcCellRef<'_, Coroutine> {
        self.coroutines
            .get(&self.current)
            .expect("NO CURRENT COROUTINE")
            .borrow()
    }
    #[inline(always)]
    pub fn current_mut(&self) -> GcCellRefMut<'_, Coroutine> {
        self.coroutines
            .get(&self.current)
            .expect("NO CURRENT COROUTINE")
            .borrow_mut()
    }
}

#[derive(Debug)]
pub struct VMContext {
    pub globals: HashMap<String, RuntimeValue>,
    pub registry: HashMap<String, RuntimeValue>,
}

/// Duka's virtual machine
#[derive(Debug)]
pub struct VM {
    ctx: VMContext,
    scheduler: Scheduler,
    pub heap: gc::Heap,
}

impl VM {
    pub fn new(/*params: Vec<RuntimeValue>*/) -> Self {
        // create heap first so we can allocate native closures into it
        let mut heap = gc::Heap::new();
        let mut globals = HashMap::new();

        globals.insert(
            "print".into(),
            RuntimeValue::NativeFunc(heap.alloc(GcCell::new(RustClosure::nonreturn(|sv, _h| {
                println!("{:?}", sv.get_stack(1));
                Ok(())
            })))),
        );

        globals.insert(
            sugar::TYPE_IS_TABLE.to_owned(),
            RuntimeValue::NativeFunc(heap.alloc(GcCell::new(RustClosure::returning::<1, _>(
                |sv, _h| {
                    let val = sv.get_stack(1)?;
                    sv.set_stack(
                        1,
                        RuntimeValue::Bool(matches!(val, RuntimeValue::Table(..))),
                    )?;
                    Ok(())
                },
            )))),
        );
        // create scheduler; heap already created above
        let scheduler = Scheduler::with_main(CoState::new(), &mut heap);

        let ctx = VMContext {
            globals,
            registry: HashMap::new(),
        };
        Self {
            ctx,
            scheduler,
            heap,
        }
    }
}

impl VM {
    /// 执行 GC
    ///
    /// 会调用所有有元方法的 Table 的 finalizer(__gc, __close)
    pub fn collect_gc(&mut self) -> Result<(), DukaRuntimeError> {
        let mut finalizers = vec![];
        self.heap.collect_with_finalizer(
            &[&self.ctx as &dyn Trace, &self.scheduler as &dyn Trace],
            |ptr| {
                let ptr = ptr as *mut RuntimeValue;
                let rv = unsafe { Box::from_raw(ptr) };
                if let RuntimeValue::Table(t) = *rv
                    && let Some(metatable) = t.borrow().metatable
                    && let Some(finalizer) = metatable
                        .borrow_mut()
                        .map
                        .get_mut(&RuntimeValue::const_str_2_runtime(MetaMethod::Gc.name()))
                    && finalizer.is_function()
                {
                    finalizers.push(std::mem::take(finalizer));
                }
            },
        );

        let mut co = self.scheduler.current_mut();
        for finalizer in finalizers {
            co.inner.append_stack(finalizer.clone())?;
            co.call(&mut self.heap, 0, 1, 0, false)?;
        }
        Ok(())
    }

    #[inline(always)]
    fn collect_if_need(&mut self) -> Result<(), DukaRuntimeError> {
        if self.heap.should_collect() {
            self.collect_gc()?;
        }
        Ok(())
    }

    #[inline]
    fn go(&mut self) -> Result<ValueCount, DukaRuntimeError> {
        self.collect_if_need()?;

        let res = self.scheduler.go(&mut self.ctx, &mut self.heap)?;

        self.collect_if_need()?;

        Ok(res)
    }
}

impl Finalize for VMContext {
    fn finalize(&self) {}
}

impl Trace for VMContext {
    fn trace(&self, tracer: &mut Tracer) {
        for v in self.globals.values() {
            v.trace(tracer);
        }
        for v in self.registry.values() {
            v.trace(tracer);
        }
    }
}

impl Finalize for Scheduler {
    fn finalize(&self) {}
}

impl Trace for Scheduler {
    fn trace(&self, tracer: &mut Tracer) {
        for c in self.coroutines.values() {
            tracer.mark(c);
        }
    }
}

impl DukaVM for VM {
    type OkType = ValueCount;

    fn execute(&mut self, proto: &DukaProto) -> Result<ValueCount, DukaRuntimeError> {
        // allocate prototype and closure on VM heap
        let proto_gc = self.heap.alloc(proto.clone());
        let closure_gc = self.heap.alloc(DukaClosure::new(proto_gc));
        self.scheduler
            .main_mut()
            .push_frame(CallFrame::new_main(closure_gc));
        self.go()
    }
}
