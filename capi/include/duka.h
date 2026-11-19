#ifndef DUKA_H
#define DUKA_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define DUKA_VERSION 1

/* Opaque module registration context, valid only during dukaopen_<name>. */
typedef struct DukaModule DukaModule;

/* Per-call frame, valid only for the duration of an exported function call. */
typedef struct DukaCall DukaCall;

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

typedef struct {
    const char* data;
    size_t len;
} DukaStr;

typedef union {
    int boolean;
    int64_t integer;
    double number;
    DukaStr string;
    void* handle;
} DukaValueData;

typedef struct {
    DukaType type;
    DukaValueData as;
} DukaValue;

/* An exported native function: return the number of values pushed with duka_ret. */
typedef int (*DukaCFunc)(DukaCall* call);

/* Plugin entry: export `int dukaopen_<name>(DukaModule* m)` from a .so/.dll. */
typedef int (*DukaOpenFunc)(DukaModule* m);

/* ---- Plugin registration (inside dukaopen_<name>) ---- */
int duka_export(DukaModule* m, const char* name, DukaCFunc fn);

/* ---- Per-call argument / return (inside an exported function) ---- */
int duka_arg_count(DukaCall* call);
DukaValue duka_arg(DukaCall* call, int index);
void duka_ret(DukaCall* call, DukaValue value);
void duka_ret_int(DukaCall* call, int64_t value);
void duka_ret_number(DukaCall* call, double value);
void duka_ret_bool(DukaCall* call, int value);
void duka_ret_string(DukaCall* call, const char* data, size_t len);
void duka_error(DukaCall* call, const char* message);

/* ---- Value constructors ---- */
DukaValue duka_value_int(int64_t value);
DukaValue duka_value_number(double value);
DukaValue duka_value_bool(int value);
DukaValue duka_value_string(const char* data, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* DUKA_H */
