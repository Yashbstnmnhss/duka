# C API for Duka

The C API for Duka.

## Different from FFI

- `FFI` is **Duka-side** interface which allows user to access functions and variables from duka.
- `CAPI` is **C-side (or C++, Rust)** interface which allows external language to provide its functions positively, so that user can simply use `require "<lib>"` to interact with it.

## For Rust

See [tests/plugin.rs](./tests/plugin.rs) and example crate [plugin_fixture](./tests/plugin_fixture/src/lib.rs) for Rust API.

## For C/C++

See the header file [here](./include/duka.h).

```h
#ifndef DUKA_H
#define DUKA_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#if defined(_WIN32)
#  define DUKA_API __declspec(dllexport)
#else
#  define DUKA_API __attribute__((visibility("default")))
#endif

#define DUKA_VERSION 1

/* Opaque engine state. Bound to the thread that created it. */
typedef struct DukaState DukaState;

/* Result/status codes. */
typedef enum {
    DUKA_OK = 0,
    DUKA_ERR_RUNTIME = 1,
    DUKA_ERR_TYPE = 2,
    DUKA_ERR_ARG = 3,
    DUKA_ERR_LOAD = 4,
    DUKA_ERR_PANIC = 5,
    DUKA_ERR_OOM = 6
} DukaStatus;

/* Value tags for the tagged union. Complex values are opaque references. */
typedef enum {
    DUKA_NIL = 0,
    DUKA_BOOL = 1,
    DUKA_INT = 2,
    DUKA_NUMBER = 3,
    DUKA_STRING = 4,
    DUKA_TABLE = 5,
    DUKA_ARRAY = 6,
    DUKA_FUNCTION = 7,
    DUKA_USERDATA = 8
} DukaType;

/* A value crossing the ABI. Strings are borrowed; copy with duka_value_dup. */
typedef struct {
    DukaType type;
    union {
        int boolean;
        int64_t integer;
        double number;
        struct { const char* data; size_t len; } string;
        void* handle;
    } as;
} DukaValue;

/* Per-call frame: valid only for the duration of a native function call. */
typedef struct DukaCall DukaCall;

/* Native function: return the number of values pushed with duka_ret. */
typedef int (*DukaCFunc)(DukaCall* call);

/* Plugin entry: export `int dukaopen_<name>(DukaState* L)` from a .so/.dll. */
typedef int (*DukaOpenFunc)(DukaState* L);

/* ---- Lifecycle ---- */
DUKA_API DukaState* duka_state_new(void);
DUKA_API void duka_state_free(DukaState* L);

/* ---- Native function registration (C -> Duka) ---- */
DUKA_API void duka_register(DukaState* L, const char* name, DukaCFunc fn);

/* ---- Per-call argument / return (inside a DukaCFunc) ---- */
DUKA_API int duka_arg_count(DukaCall* call);
DUKA_API DukaValue duka_arg(DukaCall* call, int index);
DUKA_API void duka_ret(DukaCall* call, DukaValue value);
DUKA_API void duka_ret_int(DukaCall* call, int64_t value);
DUKA_API void duka_ret_number(DukaCall* call, double value);
DUKA_API void duka_ret_bool(DukaCall* call, int value);
DUKA_API void duka_ret_string(DukaCall* call, const char* data, size_t len);

/* Raise an error from inside a native function. */
DUKA_API void duka_error(DukaCall* call, const char* message);

/* ---- Value helpers ---- */
DUKA_API DukaValue duka_value_int(int64_t value);
DUKA_API DukaValue duka_value_number(double value);
DUKA_API DukaValue duka_value_bool(int value);
DUKA_API DukaValue duka_value_string(const char* data, size_t len);
DUKA_API DukaValue duka_value_dup(DukaState* L, DukaValue value);
DUKA_API void duka_value_free(DukaState* L, DukaValue value);

/* ---- Runtime module loading (Rust side dlopen + dukaopen_<name>) ---- */
DUKA_API DukaStatus duka_load_module(DukaState* L, const char* path, const char* name);

/* ---- Running scripts / calling functions (C -> Duka) ---- */
DUKA_API DukaStatus duka_run_source(DukaState* L, const char* source, size_t len);
DUKA_API DukaStatus duka_call_global(DukaState* L, const char* name, int nargs,
                                     const DukaValue* args, DukaValue* out, int* out_count);

/* ---- Error / diagnostics ---- */
DUKA_API DukaStatus duka_last_status(DukaState* L);
DUKA_API const char* duka_last_message(DukaState* L);

#ifdef __cplusplus
}
#endif

#endif /* DUKA_H */
```
