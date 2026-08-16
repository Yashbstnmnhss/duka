# WASM Wrapper for Duka Runtime

See [backend](../backend/)

`duka-backend-wasm` is a bridge for `dukac` files running in Web

## WASM

This crate will be complied into WASM target. It is literally a `duka-backend` in WASM, (like jre, dotnet) acts as a runtime environment

## Glue

### Error Code

- `0`: Success
- `1`: Binary invalid
- `2`: Runtime error
- `3`: Data pointer is null (with no error message)

(Excepts `3`) Errors will carry its message in duka_result area

### Result

Result is in json format

```json
{
    "result": ...,
    "stdout": ...,
    "stderr": ...
}
```

`"result"` is in string (TODO)

See [here](../backend/src/vm/mod.rs) and [io](../backend/src/builtin/io.rs) for Stdout & Stderr (converted into string)

### Memory

- `duka_alloc(length) -> pointer`
- `duka_free()`

### Result

- `duka_result_ptr() -> pointer`
- `duka_result_len() -> length`

### Meta Info

- `duka_version() -> u32`: Duka binary's format version

### Module Registry

- `duka_add_module(name_pointer, name_length, data_pointer, data_length) -> error_code`
- `duka_clear_modules()`

### Standard Input

- `duka_set_input(data_pointer, data_length) -> error_code`
- `duka_clear_input()`

### Running

- `duka_run(data_pointer, data_length) -> error_code`
