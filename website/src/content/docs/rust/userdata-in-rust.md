---
title: User data in rust
description: Expose a rust struct to duka with duka_user_data!.
---

User data is a struct with a payload and an optional table.

- the payload is any rust data implementing `UserDataPayload`
- the table isn't a normal duka table, it can't be modified from duka — it only
  serves functions through `__index`, and you can override its metamethods

It looks like a table but everything lives on the rust side. It has no `Trace`,
so **GC won't track anything you put in there** — if you need GC'd values, use a
table instead.

## `duka_user_data!`

```rust
duka_user_data! {
    struct Counter { value: i64 }
    constructor fn new(value: i64) -> Self {
        Counter { value }
    }
    destructor fn drop(&mut self) -> Result<(), DukaRuntimeError> {
        Ok(())
    }
    metamethod {
        #[duka_builtin(name = "len", params(self: userdata), returns(int))]
        fn impl_len(&self) -> Result<RuntimeValue, DukaRuntimeError> {
            Ok(RuntimeValue::Int(1))
        },
    },
    #[duka_builtin(params(self: userdata), returns(int))]
    fn get(&self) -> Result<RuntimeValue, DukaRuntimeError> {
        Ok(RuntimeValue::Int(self.value))
    },
}
```

- sections (`constructor`, `destructor`, `metamethod { }`, methods) may appear in
  any order, methods are separated by `,`
- `constructor` cannot touch `self`
- `destructor` takes `&self` or `&mut self`, registered as `__gc`, run when the
  value is collected
- inside `metamethod { }` the `__` prefix is added for you, so `name = "len"`
  registers `__len`
- outside the block a name starting with `__` is a metamethod and must be a real
  one (`__index`, `__add`, `__tostring`, ...) or a csugar helper (`__bind`,
  `__return`, `__zero`, `__while`, `__forin`, `__combine`)
- when `__index` is declared the macro keeps it instead of pointing it at the
  metatable, so lookup is on you

Source: `backend/src/value.rs`, `backend/src/builtin/io.rs`.
