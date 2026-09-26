# User Data in Duka

See [RuntimeValue::UserData](../backend/src/value.rs)

See [stdlib io](../backend/src/builtin/io.rs)

## User Data

User data is a struct with a payload and an optional table.

- The payload can be any rust data(implemented trait `UserDataPayload`), it is used to hold rust data.
- The table in user data isn't normal duka table. It cannot be modified by user in duka. It only provides functions via `__index`, but you can custom its behavior by overwriting its meta methods.

## User Data & Table

User data looks like a **table** but with all rust-side data.
While duka's table is designed for storing Duka's data(also user data), user data **doesn't** support any runtime duka value with GC (No `Trace` and `tracer` for user data, GC won't work properly). **It is dangerous to use GC data in user data**. (In this case, table is better)

## duka_user_data!

`duka_user_data!` declares the struct, its metatable and its meta info.

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

- Sections(`constructor`, `destructor`, `metamethod { }`, methods) may appear in any order, methods are separated by `,`.
- `constructor` cannot access `self`.
- `destructor` takes `&self` or `&mut self` and is registered as `__gc`, the vm runs it when the value is collected.
- Inside `metamethod { }` the `__` prefix is added to the registered name, so `name = "len"` registers `__len`.
- Outside the block a registered name starting with `__` is taken as a metamethod. It must be a `MetaMethod` name(`__index`, `__add`, `__tostring`, ...) or a `csugar` helper(`__bind`, `__return`, `__zero`, `__while`, `__forin`, `__combine`), otherwise the macro fails to compile.
- `__close` compiles but the vm never dispatches it, the macro emits a deprecation warning.
- When `__index` is declared the macro keeps it instead of pointing `__index` at the metatable itself, so normal method lookup is on you then.
