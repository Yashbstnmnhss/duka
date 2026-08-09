//! Backend of Duka  
//!
//! Including codegen, binary, virtual machine, runtime value

use duka_macros::{Info, 史書云};
use duka_shared::utils::SemVer;

use crate::{errors::DukaTraceError, value::DukaProto};

pub mod builtin;
pub mod codegen;
pub mod errors;
pub mod instructions;
pub mod value;
pub mod vm;

#[derive(Info, Debug, Clone, PartialEq)]
#[non_exhaustive]
#[idcard(u8)]
pub enum SysCallId {
    Logic,
}

/// Common interface for virtual machine of Duka
pub trait DukaVM {
    type OkType;

    fn execute(&mut self, proto: &DukaProto) -> Result<Self::OkType, DukaTraceError>;
}

pub const VERSION: SemVer = 史書云! {
    <<後端>> 者
    為 世家 "項目之創立" 也
    為 世家 "Instruction之完善" 也
    為 世家 "虛擬機之創立" 也
    為 世家 "Dumplings之嘗試" 也
    為 世家 "SemVer及其宏之創立" 也
    為 列傳 "Dumplings讀寫bug" 也
};

#[cfg(test)]
mod tests {

    use std::io::Cursor;

    use crate::{
        codegen::binary::{DukaBinary, DukaDumpError, Dumplings},
        errors::DukaRuntimeError,
        instructions::Instruction,
        value::{DukaProto, MID_STR_LEN, RuntimeDukaTable, RuntimeValue, SHORT_STR_LEN},
    };
    use duka_gc::Heap;
    use duka_shared::errors::Span;
    use duka_shared::{
        ir::{UpIndex, UpValueKind},
        types::DebugInfo,
        value::ConstValue,
    };

    #[test]
    fn split_test() {
        let a: u16 = 12;
        let b: u16 = 23;
        println!("{a:b} & {b:b}");
        let r = ((a as u32) << 16) | (b as u32);
        println!("{r:b}");
        assert_eq!(a as u32, (r & ((u16::MAX as u32) << 16)) >> 16);
        assert_eq!(b as u32, r & (u16::MAX as u32));
    }

    #[test]
    fn instruction_macro_test() {
        use crate::instructions::{DecodeInstruction, Instruction as I, InstructionName};

        let i = I::Move(1, 2);
        assert_eq!(i.decode().unwrap(), DecodeInstruction::Move(1, 2));
        assert_eq!(i.name().unwrap(), InstructionName::Move);
        assert!(i.check_set_a().unwrap());
        assert!(I::validate(i.raw()));
        let i = I::LoadI(1, -2);
        assert_eq!(i.decode().unwrap(), DecodeInstruction::LoadI(1, -2));
    }

    #[test]
    fn instruction_encode_decode_test() {
        use crate::instructions::Instruction as I;

        let cases = vec![
            I::Move(0, 0),
            I::Move(255, 255),
            I::LoadI(100, -100),
            I::LoadI(0, 0),
            I::LoadTrue(50),
            I::LoadFalse(50),
            I::LoadNil(10, 5),
            I::Add(1, 2, 3),
            I::Sub(10, 20, 30),
            I::Mul(5, 5, 5),
            I::Div(100, 10, 5),
            I::Mod(10, 3, 1),
            I::Pow(2, 3, 8),
            I::Xor(1, 0, 1),
            I::BitAnd(255, 15, 15),
            I::BitOr(0, 15, 15),
            I::BitXor(5, 3, 6),
            I::ShiftL(1, 1, 2),
            I::ShiftR(4, 1, 2),
            I::Minus(1, 100),
            I::Not(1, 0),
            I::BitNot(1, 255),
            I::Length(1, 10),
            I::Concat(1, 5),
            I::NewTable(5),
            I::Return0(),
        ];

        for inst in cases {
            let raw = inst.raw();
            assert!(I::validate(raw));
            let decoded = I::from_raw(raw);
            assert_eq!(inst, decoded);
        }
    }

    #[test]
    fn instruction_jump_test() {
        use crate::instructions::{Instruction as I, SignedBits25};

        let offset: SignedBits25 = -100;
        let i = I::Jump(offset);
        assert!(I::validate(i.raw()));

        let offset2: SignedBits25 = 1000;
        let i2 = I::Jump(offset2);
        assert!(I::validate(i2.raw()));
    }

    #[test]
    fn instruction_comparison_test() {
        use crate::instructions::Instruction as I;

        let cases = vec![
            I::Equal(1, 2, 3, true),
            I::Equal(1, 2, 3, false),
            I::Less(1, 2, 3),
            I::LessEqual(1, 2, 3),
            I::EqualK(1, 2, 3, true),
            I::EqualK(1, 2, 3, false),
            I::EqualI(1, 2, -5, true),
            I::EqualI(1, 2, 5, false),
            I::LessI(1, 2, 10),
            I::LessEqualI(1, 2, 10),
            I::GreaterI(1, 2, 10),
            I::GreaterEqualI(1, 2, 10),
        ];

        for inst in cases {
            assert!(I::validate(inst.raw()));
        }
    }

    #[test]
    fn instruction_call_test() {
        use crate::instructions::Instruction as I;

        let cases = vec![
            I::Call(0, 1, 1),
            I::Call(10, 5, 3),
            I::TailCall(0, 1, 1),
            I::SysCall(0, 1, 1),
        ];

        for inst in cases {
            assert!(I::validate(inst.raw()));
        }
    }

    #[test]
    fn instruction_for_loop_test() {
        use crate::instructions::Instruction as I;

        let cases = vec![
            I::ForPrepare(0, 10),
            I::ForLoop(0, 10),
            I::TForPrepare(0, 10),
            I::TForCall(0, 5),
            I::TForLoop(0, 10),
        ];

        for inst in cases {
            assert!(I::validate(inst.raw()));
        }
    }

    #[test]
    fn instruction_closure_test() {
        use crate::instructions::Instruction as I;

        let cases = vec![
            I::Closure(0, 0),
            I::Closure(10, 100),
            I::GetUpVal(0, 5),
            I::SetUpVal(5, 0),
        ];

        for inst in cases {
            assert!(I::validate(inst.raw()));
        }
    }

    #[test]
    fn dumpling_header_test() -> Result<(), DukaDumpError> {
        use crate::codegen::binary::*;

        let header = DukaBinaryHeader {};
        let mut output: Vec<u8> = vec![];

        header.dl_write(&mut output)?;

        let header2 = DukaBinaryHeader::dl_read(&mut Cursor::new(&output))?;
        println!("{:?}", header2);

        assert_eq!(output, [68, 85, 75, 65, 1, 1, 0, 5, 1, 8, 8, 4]);
        Ok(())
    }

    #[test]
    fn dumpling_proto_test() -> Result<(), DukaDumpError> {
        let proto = DukaProto {
            up_indexes: [UpIndex {
                name: None,
                local: true,
                index: 2,
                kind: UpValueKind::Regular,
            }]
            .into(),
            constants: [ConstValue::Int(114514)].into(),
            runtime_constants: std::cell::RefCell::new(None),
            instructions: [Instruction::Move(1, 2), Instruction::Add(1, 2, 3)].into(),
            nested_protos: Box::default(),
            has_var_arg: true,
            param_count: 5,
            used_reg_count: 10,
            debug_info: Box::new(DebugInfo::default()),
            logic: None,
        };
        let binary = DukaBinary::new(proto);
        let mut output = vec![];
        binary.dl_write(&mut output)?;
        println!("{:?}", output);

        let binary2 = DukaBinary::dl_read(&mut Cursor::new(&output))?;
        assert_eq!(binary, binary2);
        Ok(())
    }

    #[test]
    fn dumpling_const_value_test() -> Result<(), DukaDumpError> {
        let values = vec![
            ConstValue::Nil,
            ConstValue::Int(0),
            ConstValue::Int(i64::MAX),
            ConstValue::Int(i64::MIN),
            ConstValue::Float(0.0),
            ConstValue::Float(std::f64::consts::PI),
            ConstValue::Float(-1.5),
            ConstValue::Bool(true),
            ConstValue::Bool(false),
            ConstValue::String(Box::new(*b"hello")),
            ConstValue::String(Box::new([])),
        ];

        for val in values {
            let mut output = vec![];
            val.dl_write(&mut output)?;
            let val2 = ConstValue::dl_read(&mut Cursor::new(&output))?;
            assert_eq!(val, val2);
        }

        Ok(())
    }

    #[test]
    fn runtime_value_nil_test() {
        let nil = RuntimeValue::Nil;
        assert!(nil.is_nil());
        assert_eq!(nil.type_name_of(), "nil");
        assert!(!nil.eval_to_bool());
        assert_eq!(nil.name(), "nil");
    }

    #[test]
    fn runtime_value_int_test() {
        let int = RuntimeValue::Int(42);
        assert!(int.is_number());
        assert_eq!(int.type_name_of(), "int");
        assert!(int.eval_to_bool());
        assert_eq!(int.eval_to_int().unwrap(), 42);
        assert_eq!(int.eval_to_float().unwrap(), 42.0);

        let neg_int = RuntimeValue::Int(-100);
        assert_eq!(neg_int.eval_to_int().unwrap(), -100);
    }

    #[test]
    fn runtime_value_float_test() {
        let float = RuntimeValue::Float(3.2);
        assert!(float.is_number());
        assert_eq!(float.type_name_of(), "float");
        assert!(float.eval_to_bool());
        assert_eq!(float.eval_to_float().unwrap(), 3.2);

        let zero = RuntimeValue::Float(0.0);
        assert!(zero.eval_to_bool());

        let neg = RuntimeValue::Float(-2.5);
        assert_eq!(neg.eval_to_float().unwrap(), -2.5);
    }

    #[test]
    fn runtime_value_bool_test() {
        let t = RuntimeValue::Bool(true);
        let f = RuntimeValue::Bool(false);

        assert_eq!(t.type_name_of(), "bool");
        assert_eq!(f.type_name_of(), "bool");

        assert!(t.eval_to_bool());
        assert!(!f.eval_to_bool());

        assert_eq!(t.eval_to_int().unwrap(), 1);
        assert_eq!(f.eval_to_int().unwrap(), 0);
        assert_eq!(t.eval_to_float().unwrap(), 1.0);
        assert_eq!(f.eval_to_float().unwrap(), 0.0);
    }

    #[test]
    fn runtime_value_short_string_test() {
        let short = RuntimeValue::from_short_str_unsafe("hello");
        assert!(short.is_string());
        assert_eq!(short.type_name_of(), "string");
        assert!(short.eval_to_bool());
        assert_eq!(short.eval_to_string(), "hello");

        let empty = RuntimeValue::from_short_str_unsafe("");
        assert!(empty.is_string());
        assert_eq!(empty.eval_to_string(), "");

        let max_short = RuntimeValue::from_short_str_unsafe("12345678901234");
        assert!(max_short.is_string());
    }

    #[test]
    fn runtime_value_string_from_heap_test() {
        let mut heap = Heap::new();

        let short = RuntimeValue::from_string(&mut heap, "hi".to_string());
        assert!(matches!(short, RuntimeValue::ShortString(2, _)));

        let mid = RuntimeValue::from_string(&mut heap, "a".repeat(SHORT_STR_LEN + 1));
        assert!(matches!(mid, RuntimeValue::MediumString(_)));

        let long = RuntimeValue::from_string(&mut heap, "a".repeat(MID_STR_LEN + 1));
        assert!(matches!(long, RuntimeValue::LongString(_)));
    }

    #[test]
    fn runtime_value_table_test() {
        let mut heap = Heap::new();
        let table = RuntimeValue::Table(heap.alloc(duka_gc::GcCell::new(RuntimeDukaTable::new(4))));

        assert!(table.is_table());
        assert_eq!(table.type_name_of(), "table");
        assert!(table.eval_to_bool());
    }

    #[test]
    fn runtime_value_from_const_test() {
        let mut heap = Heap::new();

        let nil = RuntimeValue::from_const(&mut heap, ConstValue::Nil);
        assert!(nil.is_nil());

        let int = RuntimeValue::from_const(&mut heap, ConstValue::Int(42));
        assert_eq!(int.eval_to_int().unwrap(), 42);

        let float = RuntimeValue::from_const(&mut heap, ConstValue::Float(1.5));
        assert_eq!(float.eval_to_float().unwrap(), 1.5);

        let bool_val = RuntimeValue::from_const(&mut heap, ConstValue::Bool(true));
        assert!(bool_val.eval_to_bool());

        let str_val = RuntimeValue::from_const(&mut heap, ConstValue::String(Box::new(*b"test")));
        assert!(str_val.is_string());
    }

    #[test]
    fn runtime_value_display_test() {
        assert_eq!(format!("{}", RuntimeValue::Nil), "nil");
        assert_eq!(format!("{}", RuntimeValue::Int(42)), "42");
        assert_eq!(format!("{}", RuntimeValue::Float(3.2)), "3.2");
        assert_eq!(format!("{}", RuntimeValue::Bool(true)), "true");
        assert_eq!(format!("{}", RuntimeValue::Bool(false)), "false");

        let short = RuntimeValue::from_short_str_unsafe("hello");
        assert_eq!(format!("{}", short), "hello");
    }

    #[test]
    fn runtime_value_equality_test() {
        assert_eq!(RuntimeValue::Nil, RuntimeValue::Nil);
        assert_eq!(RuntimeValue::Int(42), RuntimeValue::Int(42));
        assert_ne!(RuntimeValue::Int(42), RuntimeValue::Int(43));
        assert_eq!(RuntimeValue::Float(1.0), RuntimeValue::Float(1.0));
        assert_eq!(RuntimeValue::Bool(true), RuntimeValue::Bool(true));
        assert_eq!(RuntimeValue::Bool(false), RuntimeValue::Bool(false));
        assert_ne!(RuntimeValue::Bool(true), RuntimeValue::Bool(false));

        let s1 = RuntimeValue::from_short_str_unsafe("test");
        let s2 = RuntimeValue::from_short_str_unsafe("test");
        assert_eq!(s1, s2);
    }

    #[test]
    fn runtime_value_default_test() {
        let default = RuntimeValue::default();
        assert!(default.is_nil());
    }

    #[test]
    fn runtime_duka_table_test() {
        let mut table = RuntimeDukaTable::new(4);

        assert_eq!(table.len(), 0);

        table.set(RuntimeValue::Int(1), RuntimeValue::Int(100));
        assert_eq!(table.len(), 1);
        assert_eq!(
            table.get(&RuntimeValue::Int(1)),
            Some(&RuntimeValue::Int(100))
        );

        table.set(RuntimeValue::Int(1), RuntimeValue::Int(200));
        assert_eq!(
            table.get(&RuntimeValue::Int(1)),
            Some(&RuntimeValue::Int(200))
        );

        table.array_set(5, RuntimeValue::Int(500));
        assert_eq!(table.array_get(5), Some(&RuntimeValue::Int(500)));
        assert_eq!(table.array_get(0), None);
    }

    #[test]
    fn duka_proto_display_test() {
        let proto = DukaProto {
            up_indexes: Box::default(),
            constants: [ConstValue::Int(1)].into(),
            runtime_constants: std::cell::RefCell::new(None),
            instructions: [Instruction::Move(1, 2)].into(),
            nested_protos: Box::default(),
            has_var_arg: false,
            param_count: 2,
            used_reg_count: 5,
            debug_info: Box::new(DebugInfo::default()),
            logic: None,
        };

        let display = format!("{}", proto);
        assert!(display.contains("<Prototype>"));
        assert!(display.contains("2"));
    }

    #[test]
    fn duka_proto_with_name_test() {
        let debug_info = DebugInfo {
            inst_spans: [].into(),
            all_span: Span::EMPTY,
            debug_name: Some("test_function".into()),
        };

        let proto = DukaProto {
            up_indexes: Box::default(),
            constants: Box::default(),
            runtime_constants: std::cell::RefCell::new(None),
            instructions: Box::default(),
            nested_protos: Box::default(),
            has_var_arg: true,
            param_count: 1,
            used_reg_count: 3,
            debug_info: Box::new(debug_info),
            logic: None,
        };

        let display = format!("{}", proto);
        assert!(display.contains("test_function"));
        assert!(display.contains("..."));
    }

    #[test]
    fn syscall_id_test() {
        use crate::SysCallId;

        assert_eq!(SysCallId::Logic as u8, 0);

        let id = SysCallId::Logic;
        assert_eq!(id.name(), "logic");
    }

    #[test]
    fn vm_creation_test() {
        use crate::vm::VM;
        use duka_gc::Heap;

        let heap = Heap::new();
        let vm = VM::new(heap);

        assert!(vm.main_coroutine().inner.stack.is_empty());
    }

    #[test]
    fn scheduler_test() {
        use crate::vm::{Scheduler, coroutine::CoState};
        use duka_gc::Heap;

        let mut heap = Heap::new();
        let scheduler = Scheduler::with_main(CoState::new_unsafe(None), &mut heap);

        assert!(scheduler.main().inner.status.is_go_able());
    }

    #[test]
    fn coroutine_status_test() {
        use crate::vm::coroutine::CoroutineStatus;

        assert!(CoroutineStatus::Ready.is_go_able());
        assert!(CoroutineStatus::Suspended.is_go_able());
        assert!(!CoroutineStatus::Running.is_go_able());
        assert!(!CoroutineStatus::Dead.is_go_able());
    }

    #[test]
    fn native_api_shadow_and_gc_flag_test() {
        use crate::vm::coroutine::{CoroutineStatus, GcFlagCell, NativeApi, ShadowCell};

        let shadow: ShadowCell = std::rc::Rc::default();
        let gc_flag: GcFlagCell = std::rc::Rc::default();

        let mut api = NativeApi::with_runtime(shadow.clone(), gc_flag.clone());
        assert_eq!(api.co_status(7).name(), "unknown");

        shadow.borrow_mut().insert(7, CoroutineStatus::Suspended);
        assert_eq!(api.co_status(7).name(), "suspended");

        assert!(!gc_flag.get());
        api.request_gc();
        assert!(gc_flag.get());

        assert_eq!(
            NativeApi::default().co_status(0).name(),
            CoroutineStatus::Unknown.name()
        );
    }
    #[test]
    fn call_frame_test() {
        use crate::vm::frame::CallFrame;
        use duka_gc::Heap;

        let mut heap = Heap::new();
        let proto = DukaProto {
            up_indexes: Box::default(),
            constants: Box::default(),
            runtime_constants: std::cell::RefCell::new(None),
            instructions: Box::default(),
            nested_protos: Box::default(),
            has_var_arg: false,
            param_count: 0,
            used_reg_count: 5,
            debug_info: Box::new(DebugInfo::default()),
            logic: None,
        };

        let closure = crate::value::DukaClosure::from_proto(heap.alloc(proto));
        let gc_closure = heap.alloc(closure);

        let frame = CallFrame::main(gc_closure);

        assert_eq!(frame.pc, 0);
        assert_eq!(frame.get_base(), 0);

        let mut call_frame = CallFrame::call(10, 5, 2);
        assert_eq!(call_frame.get_base(), 10);

        call_frame.set_base(20);
        assert_eq!(call_frame.get_base(), 20);
    }

    #[test]
    fn co_state_test() {
        use crate::vm::coroutine::CoState;

        let state = CoState::new_unsafe(Some(32));
        assert!(state.stack.is_empty());
        assert!(state.frames.is_empty());
    }

    #[test]
    fn co_state_stack_operations_test() -> Result<(), DukaRuntimeError> {
        use crate::vm::coroutine::CoState;
        use duka_gc::Heap;

        let mut heap = Heap::new();
        let proto = DukaProto {
            up_indexes: Box::default(),
            constants: Box::default(),
            runtime_constants: std::cell::RefCell::new(None),
            instructions: [Instruction::LoadTrue(0)].into(),
            nested_protos: Box::default(),
            has_var_arg: false,
            param_count: 0,
            used_reg_count: 5,
            debug_info: Box::new(DebugInfo::default()),
            logic: None,
        };

        let closure = crate::value::DukaClosure::from_proto(heap.alloc(proto));
        let mut state = CoState::with_closure(heap.alloc(closure));

        assert!(state.set_stack(0, RuntimeValue::Int(42)).is_ok());
        assert_eq!(state.get_stack(0)?, &RuntimeValue::Int(42));

        assert!(state.append_stack(RuntimeValue::Bool(true)).is_ok());
        assert_eq!(state.get_stack(1)?, &RuntimeValue::Bool(true));
        Ok(())
    }

    #[test]
    fn up_value_test() {
        use crate::value::UpValue;

        let open = UpValue::Open(5);
        assert!(matches!(open, UpValue::Open(5)));

        let closed = UpValue::Closed(RuntimeValue::Int(42));
        assert!(matches!(closed, UpValue::Closed(_)));
    }

    #[test]
    fn duka_closure_test() {
        use crate::value::DukaClosure;
        use duka_gc::Heap;

        let mut heap = Heap::new();
        let proto = DukaProto {
            up_indexes: Box::default(),
            constants: Box::default(),
            runtime_constants: std::cell::RefCell::new(None),
            instructions: Box::default(),
            nested_protos: Box::default(),
            has_var_arg: false,
            param_count: 0,
            used_reg_count: 0,
            debug_info: Box::new(DebugInfo::default()),
            logic: None,
        };

        let closure = DukaClosure::from_proto(heap.alloc(proto));
        assert!(closure.up_values.is_empty());
    }

    #[test]
    fn rust_closure_test() {
        use crate::value::RustClosure;
        use duka_shared::types::ValueCount;

        let closure = RustClosure::returns(move |_c, _h, _n| Ok(ValueCount::Exact(0)), None);

        assert!(std::ptr::eq(&*closure.func, &*closure.func));
    }

    #[test]
    fn runtime_value_type_check_test() {
        let mut heap = Heap::new();

        assert!(RuntimeValue::Nil.is_nil());
        assert!(!RuntimeValue::Int(0).is_nil());

        assert!(RuntimeValue::Int(0).is_number());
        assert!(RuntimeValue::Float(0.0).is_number());
        assert!(!RuntimeValue::Bool(true).is_number());

        assert!(RuntimeValue::from_short_str_unsafe("").is_string());
        assert!(!RuntimeValue::Int(0).is_string());

        assert!(
            RuntimeValue::Table(heap.alloc(duka_gc::GcCell::new(RuntimeDukaTable::new(0))))
                .is_table()
        );
        assert!(!RuntimeValue::Nil.is_table());

        assert!(RuntimeValue::Bool(true).is_bool());
        assert!(!RuntimeValue::Int(0).is_bool());
    }

    #[test]
    fn runtime_value_eval_to_int_edge_cases_test() {
        assert_eq!(RuntimeValue::Int(i64::MAX).eval_to_int().unwrap(), i64::MAX);
        assert_eq!(RuntimeValue::Int(i64::MIN).eval_to_int().unwrap(), i64::MIN);

        assert!(RuntimeValue::Nil.eval_to_int().is_none());
        assert!(
            RuntimeValue::from_short_str_unsafe("abc")
                .eval_to_int()
                .is_none()
        );
    }

    #[test]
    fn runtime_value_eval_to_float_edge_cases_test() {
        assert!(RuntimeValue::Nil.eval_to_float().is_none());
        assert!(
            RuntimeValue::from_short_str_unsafe("abc")
                .eval_to_float()
                .is_none()
        );
    }

    #[test]
    fn runtime_value_hash_test() {
        use std::collections::HashSet;

        let mut set = HashSet::new();

        set.insert(RuntimeValue::Nil);
        set.insert(RuntimeValue::Int(42));
        set.insert(RuntimeValue::Float(3.2));
        set.insert(RuntimeValue::Bool(true));
        set.insert(RuntimeValue::from_short_str_unsafe("test"));

        assert_eq!(set.len(), 5);

        assert!(set.contains(&RuntimeValue::Nil));
        assert!(set.contains(&RuntimeValue::Int(42)));
        assert!(set.contains(&RuntimeValue::Float(3.2)));
        assert!(set.contains(&RuntimeValue::Bool(true)));
        assert!(set.contains(&RuntimeValue::from_short_str_unsafe("test")));
    }

    // ----- logic engine tests -----

    #[test]
    fn logic_single_fact() {
        use crate::codegen::logic::{CompiledQuery, LogicProto, Procedure};
        use crate::instructions::logic::LogicInstruction as I;
        use crate::vm::logic::execute_query;
        use duka_shared::types::QueryCount;

        let proto = LogicProto {
            strings: vec!["hello".into()],
            procedures: vec![Procedure {
                name: "fact".into(),
                arity: 1,
                clauses: vec![vec![I::UnifyConst(0, 0), I::Succeed()]],
            }],
            queries: vec![CompiledQuery {
                instructions: vec![I::UnifyVar(0), I::Call(0)],
                count: QueryCount::All,
            }],
        };
        let solutions = execute_query(&proto, 0).unwrap();
        assert_eq!(solutions.len(), 1);
        assert_eq!(solutions[0].get(&0).unwrap(), "hello");
    }

    #[test]
    fn logic_rule_with_body() {
        use crate::codegen::logic::{CompiledQuery, LogicProto, Procedure};
        use crate::instructions::logic::LogicInstruction as I;
        use crate::vm::logic::execute_query;
        use duka_shared::types::QueryCount;

        // parent(X,Y) :- father(X,Y).  father(john, bob).
        let proto = LogicProto {
            strings: vec!["john".into(), "bob".into()],
            procedures: vec![
                Procedure {
                    name: "parent".into(),
                    arity: 2,
                    clauses: vec![vec![
                        I::UnifyVar(0),
                        I::UnifyVar(1),
                        I::Call(1),
                        I::Proceed(),
                    ]],
                },
                Procedure {
                    name: "father".into(),
                    arity: 2,
                    clauses: vec![vec![I::UnifyConst(0, 0), I::UnifyConst(1, 1), I::Succeed()]],
                },
            ],
            queries: vec![CompiledQuery {
                instructions: vec![I::UnifyVar(0), I::UnifyVar(1), I::Call(0)],
                count: QueryCount::All,
            }],
        };
        let solutions = execute_query(&proto, 0).unwrap();
        assert_eq!(solutions.len(), 1);
        assert_eq!(solutions[0].get(&0).unwrap(), "john");
        assert_eq!(solutions[0].get(&1).unwrap(), "bob");
    }

    #[test]
    fn logic_failed_match() {
        use crate::codegen::logic::{CompiledQuery, LogicProto, Procedure};
        use crate::instructions::logic::LogicInstruction as I;
        use crate::vm::logic::execute_query;
        use duka_shared::types::QueryCount;

        // fact(hello).  ?- fact(bad).
        let proto = LogicProto {
            strings: vec!["hello".into(), "bad".into()],
            procedures: vec![Procedure {
                name: "fact".into(),
                arity: 1,
                clauses: vec![vec![I::UnifyConst(0, 0), I::Succeed()]],
            }],
            queries: vec![CompiledQuery {
                instructions: vec![I::UnifyConst(0, 1), I::Call(0)],
                count: QueryCount::All,
            }],
        };
        let solutions = execute_query(&proto, 0).unwrap();
        assert_eq!(solutions.len(), 0);
    }

    #[test]
    fn logic_multi_clause_backtrack() {
        use crate::codegen::logic::{CompiledQuery, LogicProto, Procedure};
        use crate::instructions::logic::LogicInstruction as I;
        use crate::vm::logic::execute_query;
        use duka_shared::types::QueryCount;

        // color(red). color(blue). color(green).
        // ?- color(X).  => 3 solutions
        let proto = LogicProto {
            strings: vec!["red".into(), "blue".into(), "green".into()],
            procedures: vec![Procedure {
                name: "color".into(),
                arity: 1,
                clauses: vec![
                    vec![I::UnifyConst(0, 0), I::Succeed()],
                    vec![I::UnifyConst(0, 1), I::Succeed()],
                    vec![I::UnifyConst(0, 2), I::Succeed()],
                ],
            }],
            queries: vec![CompiledQuery {
                instructions: vec![I::UnifyVar(0), I::Call(0)],
                count: QueryCount::All,
            }],
        };
        let solutions = execute_query(&proto, 0).unwrap();
        assert_eq!(solutions.len(), 3);
        let vals: Vec<&str> = solutions.iter().map(|s| s[&0].as_str()).collect();
        assert_eq!(vals, ["red", "blue", "green"]);
    }

    #[test]
    fn logic_multi_clause_partial_fail() {
        use crate::codegen::logic::{CompiledQuery, LogicProto, Procedure};
        use crate::instructions::logic::LogicInstruction as I;
        use crate::vm::logic::execute_query;
        use duka_shared::types::QueryCount;

        // rank(1). rank(3). rank(5).
        // ?- rank(3).  => matches the second clause only
        let proto = LogicProto {
            strings: vec!["1".into(), "3".into(), "5".into()],
            procedures: vec![Procedure {
                name: "rank".into(),
                arity: 1,
                clauses: vec![
                    vec![I::UnifyConst(0, 0), I::Succeed()],
                    vec![I::UnifyConst(0, 1), I::Succeed()],
                    vec![I::UnifyConst(0, 2), I::Succeed()],
                ],
            }],
            queries: vec![CompiledQuery {
                instructions: vec![I::UnifyConst(0, 1), I::Call(0)],
                count: QueryCount::All,
            }],
        };
        let solutions = execute_query(&proto, 0).unwrap();
        assert_eq!(solutions.len(), 1);
    }

    #[test]
    fn logic_no_solutions() {
        use crate::codegen::logic::CompiledQuery;
        use crate::instructions::logic::LogicInstruction as I;
        use crate::vm::logic::execute_query;
        use duka_shared::types::QueryCount;

        // query: ?- fail.  → never succeeds
        let proto = crate::codegen::logic::LogicProto {
            strings: vec![],
            procedures: vec![],
            queries: vec![CompiledQuery {
                instructions: vec![I::Fail()],
                count: QueryCount::All,
            }],
        };
        let solutions = execute_query(&proto, 0).unwrap();
        assert_eq!(solutions.len(), 0);
    }

    #[test]
    fn logic_multiple_vars() {
        use crate::codegen::logic::{CompiledQuery, LogicProto, Procedure};
        use crate::instructions::logic::LogicInstruction as I;
        use crate::vm::logic::execute_query;
        use duka_shared::types::QueryCount;

        // person(alice, 30).  ?- person(Name, Age).
        let proto = LogicProto {
            strings: vec!["alice".into(), "30".into()],
            procedures: vec![Procedure {
                name: "person".into(),
                arity: 2,
                clauses: vec![vec![I::UnifyConst(0, 0), I::UnifyConst(1, 1), I::Succeed()]],
            }],
            queries: vec![CompiledQuery {
                instructions: vec![I::UnifyVar(0), I::UnifyVar(1), I::Call(0)],
                count: QueryCount::All,
            }],
        };
        let solutions = execute_query(&proto, 0).unwrap();
        assert_eq!(solutions.len(), 1);
        assert_eq!(solutions[0].get(&0).unwrap(), "alice");
        assert_eq!(solutions[0].get(&1).unwrap(), "30");
    }

    #[test]
    fn logic_execute_query_out_of_bounds() {
        use crate::codegen::logic::LogicProto;
        use crate::vm::logic::execute_query;

        let proto = LogicProto {
            strings: vec![],
            procedures: vec![],
            queries: vec![],
        };
        let result = execute_query(&proto, 0);
        assert!(result.is_err());
    }
}
