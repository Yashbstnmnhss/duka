# Duka standard library

Everything is namespaced except core globals. Call `math.floor(x)`, never
`floor(x)`.

Full descriptions: https://yashbstnmnhss.github.io/duka/reference/

## Core globals

| Function | Signature |
| :-- | :-- |
| `print` | `print(...args)` |
| `require` | `require(pattern: string) -> any` |
| `typeof` | `typeof(val) -> string` |
| `instanceof` | `instanceof(value, target) -> bool` |
| `to_string` | `to_string(val) -> string` |
| `to_number` | `to_number(val) -> float?` (nil when not numeric) |
| `assert` | `assert(cond, msg = "Assertion failed")` |
| `error` | `error(msg = "Error")` |
| `is_error` | `is_error(ok, err) -> bool` — needs both result slots |
| `unwrap` | `unwrap(ok, value) -> ...` — raises on error |
| `expect` | `expect(value, msg = "Got nil value") -> any` — raises on nil |
| `try` | `try(f, ...) -> (true, ...) \| (false, message)` |
| `select` | `select(pat, ...vals) -> ...` — `pat` is an index, an array of indices, or `"#"` for the count |
| `pairs` | `pairs(tab)` |
| `ipairs` | `ipairs(tab)` |
| `clone` | `clone(val) -> any` |
| `curry` | `curry(f, ...args)` |
| `get_metatable` | `get_metatable(tab)` |
| `set_metatable` | `set_metatable(tab, mt) -> table` |
| `costatus` | `costatus(coroutine)` |

```duka
assert(to_number("2.5") == 2.5)
assert(to_string(2.5) == "2.5")
assert(typeof(1) == "int")
assert(is_error(false, "boom"))
```

## math

`abs`, `floor`, `ceil`, `round`, `sqrt`, `clamp(x, lo, hi)`, `sign`,
`max(...)`, `min(...)`, `sum(...)`, `log(x, base)`, `ln`, `log2`, `log10`,
`exp`, `hypot`, `fmod`, `modf(x) -> int, float`, `factors(n)`, `sin`, `cos`,
`tan`, `arcsin`, `arccos`, `arctan`, `arctan2`, `deg_to_rad`, `rad_to_deg`,
`randf`, `randi`, `randf_range`, `randi_range`, `set_seed`, `is_nan`, `is_inf`.

Constants: `PI`, `E`, `FLOAT_MAX`, `INT_MAX`, `INF`, `NAN`.

```duka
assert(math.floor(3.7) == 3)
assert(math.clamp(15, 0, 10) == 10)
assert(math.max(1, 5, 3) == 5)
```

## string

`len`, `upper`, `lower`, `reverse`, `repeat(s, n, sep = "")`,
`trim`, `trim_start`, `trim_end`, `find(s, sub, from = 0) -> int | nil`,
`substr(s, start, count = -1)`, `slice(s, start, end = #s)`,
`split(s, sep = " ") -> table`, `concat(...vals)`.

```duka
assert(string.upper("hi") == "HI")
assert(string.slice("hello", 0, 3) == "hel")
assert(string.split("a,b,c", ",")[1] == "b")
assert(string.find("hello", "ll") == 2)
```

## array

Arrays are 0-based.

`pack(...vals) -> array`, `unpack(arr) -> ...`, `push(arr, val)`,
`pop(arr)`, `has(arr, val) -> bool`, `insert(arr, index, value)`,
`remove(arr, index)`, `len(arr)`, `concat(arr, other)`,
`sort(arr, cmp = nil)`, `index_of(arr, who) -> int`.

```duka
local a = [3, 1, 2]
array.sort(a)
array.push(a, 4)
assert(a[0] == 1)
assert(#a == 4)
assert(array.has(a, 3))
```

## table

`keys(tab) -> array`, `values(tab) -> array`, `has(tab, key) -> bool`,
`merge(tab, other, keep = false)`, `remove(tab, key)`, `clear(tab)`,
`raw_get`, `raw_set`, `raw_get_set`, `has_value`, `capacity`.

```duka
local t = { a = 1, b = 2 }
local ks = table.keys(t)
assert(#ks == 2)
assert(table.has(t, "a"))
```

## iter

`range(from, to, step = 1)`, `repeat(who, times)`,
`map(coll, f)`, `filter(coll, pred)`, `take(coll, n)`, `skip(coll, n)`,
`chain(coll, other)`, `enumerate(coll)`, `count(coll)`,
`for_each(coll, f)`, `all(coll, pred)`, `any(coll, pred)`,
`partition(coll, pred) -> array, array`, `to_array(coll)`.

```duka
local doubled = iter.to_array(iter.map([1, 2, 3], fn(x) x * 2))
assert(doubled[0] == 2)
assert(iter.count(iter.range(0, 3)) == 3)
```

## regex

`replace_all(pattern, text, replacement, from = 0) -> string`
`replace(pattern, text, replacement, from = 0, times = 1) -> string`
`find_all(pattern, text) -> array, array`
`search(pattern, text, from = 0) -> (bool, captures...)`
`is_match(pattern, text, from = 0) -> bool`
`compile(pattern)`, `escape(pattern) -> string`

```duka
assert(regex.is_match("\\d+", "abc123"))
assert(regex.replace_all("\\s+", "a  b", "-") == "a-b")
assert(regex.replace("\\s+", "a  b  c", "-") == "a-b  c")

local re = regex.compile("a+")
assert(re:is_match("baa"))            -- methods take the text first
```

`search` and `is_match` scan forward from `from`, so a match anywhere in the
text counts.

## os

`execute(cmd)`, `exit(code = 0)`, `remove(path) -> ...`,
`rename(path, name) -> ...`, `clock() -> float | nil`, `time() -> int`,
`date() -> table`.

## io

`open(path, mode = "r") -> file, err`; `tmpfile()`; `io.stdout`,
`io.stderr`, `io.stdin`.

## json

`json.parse(text) -> value`; `json.stringify(value) -> string`.

```duka
local data = json.parse("{\"n\": 1}")
assert(data.n == 1)
```

## Type context

Compile-time type functions (`Error`, `Assert`, `Union`, `Super`, `IsSubType`,
`Regex`, `Stringify`, ...) take types and are evaluated in type position. See
https://yashbstnmnhss.github.io/duka/reference/type-context/ — rarely needed
for application code.
