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
