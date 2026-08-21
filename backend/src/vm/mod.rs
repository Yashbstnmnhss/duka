use std::any::TypeId;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::{
    DukaVM, builtin,
    errors::{DukaRuntimeError, DukaTraceError},
    instructions::{Address, Bits25},
    value::{
        DukaClosure, DukaProto, RuntimeDukaTable, RuntimeValue, RustClosure, UpValue, UserData,
    },
    vm::{
        coroutine::{
            CoState, Coroutine, CoroutineID, CoroutineStatus, GcFlagCell, InputCell, NativeApi,
            OutputCell, ShadowCell,
        },
        frame::CallFrame,
    },
};
use duka_gc::prelude::*;
use duka_gc::{Finalize, Trace, Tracer};
use duka_shared::{
    constants::{MetaMethod, csugar, ctype},
    types::ValueCount,
};

pub mod coroutine;
pub mod frame;
pub mod logic;

#[derive(Debug)]
/// Action of a running coroutine
pub enum CoAction {
    /// Return values, coroutine dead
    Return(Address, ValueCount),
    /// Yield and suspend coroutine
    Yield(Address, ValueCount),
    /// Call another coroutine
    Go(CoroutineID, Address, ValueCount, Address),
    /// Create a new coroutine
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
    shadow: ShadowCell,                                      // shadow of status of coroutines
    gc_flag: GcFlagCell,                                     // GC request flag
    stdout: Option<OutputCell>, // capture sink for print/print! invocations
    stderr: Option<OutputCell>,
    input: Option<InputCell>,
    globals: Gc<GcCell<RuntimeDukaTable>>,
    module_cache: Gc<GcCell<RuntimeDukaTable>>,
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

    fn create_main(state: CoState) -> Coroutine {
        Coroutine::new(Self::MAIN_ID, state, None)
    }

    /// ### This will create an initial coroutine *(main coroutine)* with `id = MAIN_ID`
    pub fn with_main(
        main: CoState,
        heap: &mut Heap,
        globals: Gc<GcCell<RuntimeDukaTable>>,
        module_cache: Gc<GcCell<RuntimeDukaTable>>,
    ) -> Self {
        let mut coroutines = HashMap::new();
        coroutines.insert(
            Self::MAIN_ID,
            heap.alloc(GcCell::new(Self::create_main(main))),
        );

        let shadow: ShadowCell = std::rc::Rc::default();
        shadow
            .borrow_mut()
            .insert(Self::MAIN_ID, CoroutineStatus::Ready);

        Self {
            free_list: vec![],
            id_sp: coroutines.len(),
            current: Self::MAIN_ID,
            coroutines,
            shadow,
            gc_flag: std::rc::Rc::default(),
            stdout: None,
            stderr: None,
            input: None,
            globals,
            module_cache,
        }
    }

    /// 同步
    fn refresh_shadow(&self) {
        let mut table = self.shadow.borrow_mut();
        table.clear();
        for (id, co) in &self.coroutines {
            table.insert(*id, co.borrow().inner.status);
        }
    }
    /// 请求GC的标志位
    fn take_gc_request(&mut self) -> bool {
        self.gc_flag.replace(false)
    }
    /// 设置stdout
    pub fn set_stderr(&mut self, sink: Option<OutputCell>) {
        self.stderr = sink;
    }
    /// 取出stdout
    pub fn take_stderr(&mut self) -> Option<OutputCell> {
        self.stderr.take()
    }
    /// 设置stdout
    pub fn set_stdout(&mut self, sink: Option<OutputCell>) {
        self.stdout = sink;
    }
    /// 取出stdout
    pub fn take_stdout(&mut self) -> Option<OutputCell> {
        self.stdout.take()
    }
    pub fn set_input(&mut self, cell: Option<InputCell>) {
        self.input = cell;
    }
    pub fn take_input(&mut self) -> Option<InputCell> {
        self.input.take()
    }
    /// 执行GC
    fn collect_gc(&mut self, heap: &mut Heap) -> Result<(), DukaRuntimeError> {
        let mut finalizers = vec![];
        heap.collect_with_finalizer(&[&*self as &dyn Trace], |ptr, type_id| {
            let finalizer = if type_id == TypeId::of::<GcCell<RuntimeDukaTable>>() {
                let cell = unsafe { &*(ptr as *const GcCell<RuntimeDukaTable>) };
                let table = unsafe { cell.get() };
                let Some(metatable) = table.metatable else {
                    return;
                };
                let Some(finalizer) = metatable
                    .borrow_mut()
                    .inner
                    .get_mut(&RuntimeValue::from_short_str_unsafe(MetaMethod::Gc.name()))
                    .cloned()
                else {
                    return;
                };
                finalizer
            } else if type_id == TypeId::of::<GcCell<UserData>>() {
                let cell = unsafe { &*(ptr as *const GcCell<UserData>) };
                let data = unsafe { cell.get() };
                let Some(metatable) = data.metatable else {
                    return;
                };
                let Some(finalizer) = metatable
                    .borrow_mut()
                    .inner
                    .get_mut(&RuntimeValue::from_short_str_unsafe(MetaMethod::Gc.name()))
                    .cloned()
                else {
                    return;
                };
                finalizer
            } else {
                return;
            };
            if finalizer.is_function() {
                finalizers.push(finalizer);
            }
        });

        if finalizers.is_empty() {
            return Ok(());
        }
        let mut api = NativeApi::default();
        let mut co = self.current_mut();
        for finalizer in finalizers {
            co.inner.append_stack(finalizer.clone())?;
            co.call(heap, &mut api, 0, 1u8.into(), 0u8.into(), false)?;
        }
        Ok(())
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
        let cor = Coroutine::new(id, state, Some(self.current));
        self.coroutines.insert(id, heap.alloc(GcCell::new(cor)));
        id
    }

    pub fn destroy(&mut self, id: CoroutineID) {
        if self.coroutines.remove(&id).is_some() {
            self.free_list.push(id);
        }
    }

    /// ### main loop
    pub fn go(&mut self, heap: &mut Heap) -> Result<ValueCount, DukaTraceError> {
        use CoAction::*;
        Ok(loop {
            self.refresh_shadow();
            if self.take_gc_request() {
                self.collect_gc(heap).map_err(|kind| DukaTraceError {
                    kind,
                    trace: self.current().inner.create_trace(),
                })?;
                continue;
            }
            let mut api = NativeApi::with_runtime(
                self.shadow.clone(),
                self.gc_flag.clone(),
                self.stdout.clone(),
                self.stderr.clone(),
                Some(self.globals),
                Some(self.module_cache),
                self.input.clone(),
            );
            let action = match self.current_mut().inner.execute(heap, &mut api, None) {
                Ok(a) => a,
                Err(kind) => {
                    if self.is_main() {
                        let trace = self.current().inner.create_trace();
                        return Err(DukaTraceError { kind, trace });
                    } else {
                        let id = self.current;
                        let ret = self.current().inner.ret_slot as usize;

                        self.switch_parent();
                        // See docs/stdlib.md
                        self.write_back(
                            ret,
                            vec![
                                RuntimeValue::Bool(false), //约定 [success?, ...] <- [false, error_message]
                                RuntimeValue::from_string(heap, kind.to_string()),
                            ],
                        );

                        self.destroy(id);
                        continue;
                    }
                }
            };
            match action {
                Return(from, return_count) => {
                    if self.is_main() {
                        self.main_mut().inner.status = CoroutineStatus::Ready;
                        break return_count;
                    }
                    let id = self.current;
                    let ret = self.current().inner.ret_slot as usize;

                    let mut results = vec![RuntimeValue::Bool(true)];
                    results.extend(
                        self.current_mut()
                            .inner
                            .cut_stack(from as usize, return_count.clone()),
                    );

                    self.switch_parent();
                    self.write_back(ret, results);

                    self.destroy(id);
                }
                Yield(from, yield_count) => {
                    if self.is_main() {
                        self.main_mut().inner.status = CoroutineStatus::Suspended;
                        break yield_count;
                    }
                    let ret = self.current().inner.ret_slot as usize;

                    let mut yielded = self
                        .current_mut()
                        .inner
                        .cut_stack(from as usize, yield_count.clone());
                    yielded.insert(0, RuntimeValue::Bool(true));

                    self.switch_parent();
                    self.write_back(ret, yielded);
                }
                Go(co, from, params_count, ret_slot) => {
                    let params = self
                        .current_mut()
                        .inner
                        .cut_stack(from as usize, params_count);
                    let resume = self
                        .coroutines
                        .get(&co)
                        .and_then(|c| c.borrow_mut().inner.resume_slot.take());
                    if let Some(slot) = resume {
                        let mut target = self.coroutines[&co].borrow_mut();
                        for (i, v) in params.into_iter().enumerate() {
                            target.inner.set_stack(slot as usize + i, v).ok();
                        }
                    } else {
                        self.coroutines[&co].borrow_mut().inner.stack.extend(params);
                    }
                    if let Some(target) = self.coroutines.get_mut(&co) {
                        target.borrow_mut().inner.ret_slot = ret_slot as u8;
                    }
                    self.switch(co);
                }
                Spawn(ad, from) => {
                    let closure = match self.current().inner.get_stack(from as usize) {
                        Ok(RuntimeValue::UserFunc(c)) => *c,
                        Ok(_) => {
                            return Err(DukaTraceError {
                                kind: DukaRuntimeError::InvalidValueType(ctype::CLO),
                                trace: self.current().inner.create_trace(),
                            });
                        }
                        Err(kind) => {
                            return Err(DukaTraceError {
                                kind,
                                trace: self.current().inner.create_trace(),
                            });
                        }
                    };
                    let id = self.create(CoState::with_closure(closure), heap);
                    self.current_mut()
                        .inner
                        .set_stack(ad as usize, RuntimeValue::Coroutine(id))
                        .map_err(|kind| DukaTraceError {
                            kind,
                            trace: self.current().inner.create_trace(),
                        })?;
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
            self.current_mut().inner.status = CoroutineStatus::Suspended;
            self.current = to;
            self.current_mut().inner.status = CoroutineStatus::Running;
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

    #[inline]
    pub fn main(&self) -> GcCellRef<'_, Coroutine> {
        self.coroutines
            .get(&Self::MAIN_ID)
            .expect("NO MAIN COROUTINE")
            .borrow()
    }
    #[inline]
    pub fn main_mut(&self) -> GcCellRefMut<'_, Coroutine> {
        self.coroutines
            .get(&Self::MAIN_ID)
            .expect("NO MAIN COROUTINE")
            .borrow_mut()
    }

    #[inline]
    pub fn current(&self) -> GcCellRef<'_, Coroutine> {
        self.coroutines
            .get(&self.current)
            .expect("NO CURRENT COROUTINE")
            .borrow()
    }
    #[inline]
    pub fn current_mut(&self) -> GcCellRefMut<'_, Coroutine> {
        self.coroutines
            .get(&self.current)
            .expect("NO CURRENT COROUTINE")
            .borrow_mut()
    }

    fn write_back(&mut self, slot: usize, values: Vec<RuntimeValue>) {
        let mut cur = self.current_mut();
        for (i, v) in values.into_iter().enumerate() {
            let _ = cur.inner.set_stack(slot + i, v);
        }
    }
}

#[derive(Debug, Clone)]
pub struct VMContext {
    globals: Gc<GcCell<RuntimeDukaTable>>,
}
impl VMContext {
    pub fn new(heap: &mut Heap) -> Self {
        Self {
            globals: heap.alloc(GcCell::new(RuntimeDukaTable::new(0))),
        }
    }
    pub fn register_func(&mut self, heap: &mut Heap, name: impl Into<String>, func: RustClosure) {
        let val = RuntimeValue::NativeFunc(heap.alloc(GcCell::new(func)));
        self.globals.borrow_mut().set_by_key(heap, name.into(), val);
    }
    pub fn register_table(
        &mut self,
        heap: &mut Heap,
        name: impl Into<String>,
        table: RuntimeDukaTable,
    ) {
        let val = RuntimeValue::Table(heap.alloc(GcCell::new(table)));
        self.globals.borrow_mut().set_by_key(heap, name.into(), val);
    }
}

impl Finalize for VMContext {
    fn finalize(&self) {}
}
impl Trace for VMContext {
    fn trace(&self, tracer: &mut Tracer) {
        self.globals.trace(tracer);
    }
}

/// Duka's virtual machine
#[derive(Debug)]
pub struct VM {
    vm_ctx: VMContext,
    pub scheduler: Scheduler,
    pub heap: Heap,
}

impl VM {
    pub fn new(mut heap: Heap) -> Self {
        let mut vm_globals = VMContext::new(&mut heap);

        builtin::register_all(&mut heap, &mut vm_globals);

        vm_globals.register_func(
            &mut heap,
            csugar::TYPE_IS_TABLE_ARRAY,
            RustClosure::returning::<1, _>(|sv, _h, _n| {
                let val = sv.get_stack(1)?;
                sv.set_stack(
                    1,
                    RuntimeValue::Bool(matches!(
                        val,
                        RuntimeValue::Table(..) | RuntimeValue::Array(..)
                    )),
                )?;
                Ok(())
            }),
        );
        vm_globals.register_func(
            &mut heap,
            csugar::TYPE_IS_TABLE,
            RustClosure::returning::<1, _>(|sv, _h, _n| {
                let val = sv.get_stack(1)?;
                sv.set_stack(
                    1,
                    RuntimeValue::Bool(matches!(val, RuntimeValue::Table(..))),
                )?;
                Ok(())
            }),
        );

        let module_cache = heap.alloc(GcCell::new(RuntimeDukaTable::new(0)));
        let scheduler = Scheduler::with_main(
            CoState::new_unsafe(None),
            &mut heap,
            vm_globals.globals,
            module_cache,
        );

        Self {
            vm_ctx: vm_globals,
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
        self.scheduler.collect_gc(&mut self.heap)
    }

    #[inline(always)]
    fn collect_if_need(&mut self) -> Result<(), DukaRuntimeError> {
        if self.heap.should_collect() {
            self.collect_gc()?;
        }
        Ok(())
    }

    pub fn main_coroutine(&self) -> GcCellRef<'_, Coroutine> {
        self.scheduler.main()
    }
    pub fn main_coroutine_mut(&self) -> GcCellRefMut<'_, Coroutine> {
        self.scheduler.main_mut()
    }

    pub fn set_stderr(&mut self, sink: Option<OutputCell>) {
        self.scheduler.set_stderr(sink);
    }
    pub fn take_stderr(&mut self) -> Option<OutputCell> {
        self.scheduler.take_stderr()
    }
    /// 当存在output cell时 将不会直接print, 而会写入此Cell中
    pub fn set_stdout(&mut self, sink: Option<OutputCell>) {
        self.scheduler.set_stdout(sink);
    }
    pub fn take_stdout(&mut self) -> Option<OutputCell> {
        self.scheduler.take_stdout()
    }

    pub fn set_input(&mut self, cell: Option<InputCell>) {
        self.scheduler.set_input(cell);
    }
    pub fn take_input(&mut self) -> Option<InputCell> {
        self.scheduler.take_input()
    }

    pub fn set_main_args(&mut self, args: &[RuntimeValue]) {
        self.main_coroutine_mut()
            .inner
            .stack
            .extend_from_slice(args);
    }

    /// Start from here
    pub fn set_entry_path(&mut self, path: PathBuf) {
        self.main_coroutine_mut().inner.push_module_path(path);
    }

    #[inline]
    fn go(&mut self) -> Result<ValueCount, DukaTraceError> {
        self.collect_if_need().map_err(|kind| DukaTraceError {
            kind,
            trace: Default::default(),
        })?;

        let res = self.scheduler.go(&mut self.heap)?;
        self.collect_if_need().map_err(|kind| DukaTraceError {
            kind,
            trace: Default::default(),
        })?;

        Ok(res)
    }

    pub fn take_stack(&mut self, at: usize) -> Result<RuntimeValue, DukaRuntimeError> {
        self.main_coroutine_mut().inner.take_stack(at)
    }
    pub fn stack_size(&self) -> usize {
        self.main_coroutine().inner.stack.len()
    }

    pub fn reset_main(&mut self) -> Result<(), DukaRuntimeError> {
        self.main_coroutine_mut().reset();
        self.collect_gc()
    }

    /// Run a proto immediate, take its results or error
    pub fn run_take<const C: usize>(
        proto: &DukaProto,
    ) -> Result<[RuntimeValue; C], DukaTraceError> {
        let mut vm = Self::new(Heap::new());
        let count = vm.execute(proto)?;
        let mut main = vm.main_coroutine_mut();
        let mut state = std::mem::take(&mut main.inner);
        let mut iter = state
            .take_stack_many(0, count)
            .map_err(|kind| DukaTraceError {
                kind,
                trace: Default::default(),
            })?
            .into_iter()
            .chain(std::iter::repeat(RuntimeValue::default()));
        Ok(std::array::from_fn(|_| iter.next().unwrap()))
    }
    pub fn run(proto: &DukaProto) -> Result<Box<[RuntimeValue]>, DukaTraceError> {
        let mut vm = Self::new(Heap::new());
        let count = vm.execute(proto)?;
        let mut main = vm.main_coroutine_mut();
        let mut state = std::mem::take(&mut main.inner);
        state
            .take_stack_many(0, count)
            .map_err(|kind| DukaTraceError {
                kind,
                trace: Default::default(),
            })
    }

    pub fn set_global(&mut self, key: impl Into<String>, value: impl Into<RuntimeValue>) {
        self.vm_ctx.globals.borrow_mut().set(
            RuntimeValue::from_string(&mut self.heap, key.into()),
            value.into(),
        );
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
        self.globals.trace(tracer);
        self.module_cache.trace(tracer);
    }
}

impl DukaVM for VM {
    type OkType = ValueCount;

    fn execute(&mut self, proto: &DukaProto) -> Result<ValueCount, DukaTraceError> {
        let proto_gc = self.heap.alloc(proto.clone());
        // the _ENV
        let closure = DukaClosure::from_proto(proto_gc).set_up_value(
            &mut self.heap,
            UpValue::Closed(RuntimeValue::Table(self.vm_ctx.globals)),
        );
        let closure_gc = self.heap.alloc(closure);

        self.scheduler
            .main_mut()
            .push_frame(CallFrame::main(closure_gc));
        match self.go() {
            Ok(count) => Ok(count),
            // VM can be reused after errors, e.g. by the sequential REPL.
            Err(e) => {
                self.scheduler.main_mut().inner.reset();
                Err(e)
            }
        }
    }
}
