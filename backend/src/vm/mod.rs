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
use gc::{Gc, GcCell, GcCellRef, GcCellRefMut};
use gc_derive::{Finalize, Trace};

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
            Gc::new(GcCell::new(Coroutine::new(Self::MAIN_ID, main, None))),
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
            parent: Some(self.current),

            last_wanted: 0,
        };
        self.coroutines.insert(id, Gc::new(GcCell::new(cor)));
        id
    }

    pub fn destroy(&mut self, id: CoroutineID) {
        if self.coroutines.remove(&id).is_some() {
            self.free_list.push(id);
        }
    }

    /// ### main loop
    pub fn go(&mut self, ctx: &mut VMContext) -> Result<ValueCount, DukaRuntimeError> {
        use CoAction::*;
        Ok(loop {
            let result = self.current_mut().execute(ctx)?;
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
                        _ => return Err(DukaRuntimeError::InvalidValueType("closure")),
                    };
                    let id = self.create(CoState::from_closure(closure));
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

#[derive(Debug, Trace, Finalize)]
pub struct VMContext {
    pub globals: HashMap<String, RuntimeValue>,
    pub registry: HashMap<String, RuntimeValue>,
}

/// Duka's virtual machine
#[derive(Debug, Trace, Finalize)]
pub struct VM {
    ctx: VMContext,
    scheduler: Scheduler,
}

impl VM {
    pub fn new(/*params: Vec<RuntimeValue>*/) -> Self {
        let mut globals = HashMap::new();

        globals.insert(
            "print".into(),
            RustClosure::nonreturn(|sv| {
                println!("{:?}", sv.get_stack(1));
                Ok(())
            })
            .into(),
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

        let scheduler = Scheduler::with_main(CoState::new());

        let ctx = VMContext {
            globals,
            registry: HashMap::new(),
        };
        Self { ctx, scheduler }
    }
}

impl VM {
    #[inline(always)]
    fn go(&mut self) -> Result<ValueCount, DukaRuntimeError> {
        self.scheduler.go(&mut self.ctx)
    }
}

impl DukaVM for VM {
    type OkType = ValueCount;

    fn execute(&mut self, proto: &DukaProto) -> Result<ValueCount, DukaRuntimeError> {
        let proto = Gc::new(proto.clone());
        self.scheduler
            .main_mut()
            .push_frame(CallFrame::new_main(Gc::new(DukaClosure::new(proto))));
        self.go()
    }
}
