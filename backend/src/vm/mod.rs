use std::collections::HashMap;

use crate::{
    DukaVM,
    error::DukaRuntimeError,
    instructions::Bits25,
    value::{DukaProto, RuntimeValue, RustClosure, ValueCount},
    vm::{
        coroutine::{CoState, Coroutine, CoroutineID, CoroutineStatus},
        frame::CallFrame,
    },
};
use gc::{Gc, GcCell, GcCellRef, GcCellRefMut};
use gc_derive::{Finalize, Trace};

pub mod coroutine;
pub mod frame;

/// Result of a running coroutine
pub enum ExecuteResult {
    /// Return values, coroutine dead
    Return(ValueCount),
    /// Yield and suspend coroutine
    Yield(ValueCount),
    /// Call another coroutine
    Become(),
}

/// # 协程调度器
/// 管理所有协程及其创建、切换、销毁
#[derive(Debug, Trace, Finalize)]
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
    pub fn with_main(main: CoState) -> Self {
        let mut coroutines = HashMap::new();
        coroutines.insert(
            Self::MAIN_ID,
            Gc::new(GcCell::new(Coroutine::new(Self::MAIN_ID, main))),
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
    pub fn create_switch(&mut self, state: CoState) -> CoroutineID {
        let id = self.create(state);
        self.switch(id);
        id
    }

    /// ### Create a coroutine with its CoState, returning its ID
    pub fn create(&mut self, state: CoState) -> CoroutineID {
        let id = self.gen_id();
        let cor = Coroutine {
            inner: state,
            status: CoroutineStatus::Ready,
            id,
        };
        self.coroutines.insert(id, Gc::new(GcCell::new(cor)));
        id
    }

    /// ### Remove a coroutine by its ID
    pub fn destroy(&mut self, target: CoroutineID) {
        if target == Self::MAIN_ID {
            // destroy the main coroutine is banned
            return;
        }
        if self.coroutines.remove(&target).is_some() {
            self.free_list.push(target);
        }
    }

    /// ### `go something(...)`
    #[inline(always)]
    pub fn go(&self) -> Result<ExecuteResult, DukaRuntimeError> {
        self.current_mut().execute()
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

    /// ### switch to the main coroutine
    #[inline(always)]
    pub fn switch_main(&mut self) {
        self.switch(Self::MAIN_ID);
    }

    #[inline(always)]
    pub fn main(&self) -> GcCellRef<Coroutine> {
        self.coroutines
            .get(&Self::MAIN_ID)
            .expect("NO MAIN COROUTINE")
            .borrow()
    }
    #[inline(always)]
    pub fn main_mut(&self) -> GcCellRefMut<Coroutine> {
        self.coroutines
            .get(&Self::MAIN_ID)
            .expect("NO MAIN COROUTINE")
            .borrow_mut()
    }

    #[inline(always)]
    pub fn current(&self) -> GcCellRef<Coroutine> {
        self.coroutines
            .get(&self.current)
            .expect("NO SUCH COROUTINE")
            .borrow()
    }
    #[inline(always)]
    pub fn current_mut(&self) -> GcCellRefMut<Coroutine> {
        self.coroutines
            .get(&self.current)
            .expect("NO SUCH COROUTINE")
            .borrow_mut()
    }
}

/// Duka's virtual machine
#[derive(Debug, Trace, Finalize)]
pub struct VM {
    /// 全局变量
    globals: HashMap<String, RuntimeValue>,
    /// (仅Rust) 注册表
    registry: HashMap<String, RuntimeValue>,
    /// 协程调度器
    scheduler: Scheduler,
}

impl VM {
    pub fn new(/*params: Vec<RuntimeValue>*/) -> Self {
        let mut globals = HashMap::new();

        globals.insert(
            "print".into(),
            RuntimeValue::NativeFunc(RustClosure::from_func(|sv| Ok(ValueCount::VarArg))),
        );
        // globals.insert(
        //     "print".into(),
        //     ConstValue::Func(|s| {
        //         println!("{}", s.get_stack(1));
        //         0
        //     }),
        // );
        // globals.insert(
        //     sugar::TYPE_IS_TABLE.into(),
        //     ConstValue::Func(|s| {
        //         let res = matches!(s.get_stack(1), ConstValue::Table(_));
        //         s.set_stack(0, ConstValue::Bool(res));
        //         1
        //     }),
        // );

        Self {
            globals,
            registry: HashMap::new(),
            scheduler: Scheduler::with_main(CoState::new()),
        }
    }
}

impl VM {
    fn execute_current(&mut self) -> Result<ValueCount, DukaRuntimeError> {
        Ok(loop {
            match self.scheduler.go()? {
                ExecuteResult::Return(v) => break v,
                ExecuteResult::Yield(count) => break count,
                ExecuteResult::Become() => {
                    self.scheduler.switch(114514);
                    todo!()
                }
            }
        })
    }
}

impl DukaVM for VM {
    type OkType = ValueCount;

    fn execute(&mut self, proto: &DukaProto) -> Result<ValueCount, DukaRuntimeError> {
        let proto = Gc::new(proto.clone());
        self.scheduler
            .main_mut()
            .push_frame(CallFrame::new_main(proto));
        self.execute_current()
    }
}
