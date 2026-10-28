---
title: core
description: Core builtins.
---

<a id="core"></a>

## Contents

[select](#select)

[require](#require)

[print](#print)

[typeof](#typeof)

[instanceof](#instanceof)

[to_string](#to_string)

[to_number](#to_number)

[assert](#assert)

[error](#error)

[is_error](#is_error)

[unwrap](#unwrap)

[expect](#expect)

[get_metatable](#get_metatable)

[set_metatable](#set_metatable)

[pairs](#pairs)

[ipairs](#ipairs)

[costatus](#costatus)

[try](#try)

[clone](#clone)

[curry](#curry)

[Result](#result)

## Members

<a id="select"></a>

### `select(pat: any, ...vals: any) -> ...`

> Select element(s) or length from var args

#### Params

| Name      | Type  | VarArg? | Optional? | Default    | Doc |
| :-------- | :---- | :------ | :-------- | :--------- | :-- |
| `pat`     | `any` | _false_ | _false_   | _required_ | -   |
| `...vals` | `any` | _true_  | _false_   | -          | -   |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

#### Example

```lua
... |$> select(2)   -- [...][2]
```

<a id="require"></a>

### `require(pattern: string) -> any`

> Import module by pattern

Flags: `@returns(module), @keywordish()`

#### Params

| Name      | Type     | VarArg? | Optional? | Default    | Doc |
| :-------- | :------- | :------ | :-------- | :--------- | :-- |
| `pattern` | `string` | _false_ | _false_   | _required_ | -   |

#### Returns

`any`<br/>

| Index | Type  |
| :---- | :---- |
| 0     | `any` |

<a id="print"></a>

### `print(...args: any)`

> Prints to standard output

#### Params

| Name      | Type  | VarArg? | Optional? | Default | Doc |
| :-------- | :---- | :------ | :-------- | :------ | :-- |
| `...args` | `any` | _true_  | _false_   | -       | -   |

<a id="typeof"></a>

### `typeof(val: any)`

> Get type name of value

Flags: `@keywordish()`

#### Params

| Name  | Type  | VarArg? | Optional? | Default    | Doc |
| :---- | :---- | :------ | :-------- | :--------- | :-- |
| `val` | `any` | _false_ | _false_   | _required_ | -   |

<a id="instanceof"></a>

### `instanceof(value: any, target: any)`

> Check if the value is an instance of target

Flags: `@keywordish()`

#### Params

| Name     | Type  | VarArg? | Optional? | Default    | Doc |
| :------- | :---- | :------ | :-------- | :--------- | :-- |
| `value`  | `any` | _false_ | _false_   | _required_ | -   |
| `target` | `any` | _false_ | _false_   | _required_ | -   |

<a id="to_string"></a>

### `to_string(val: any)`

> Convert to string

#### Params

| Name  | Type  | VarArg? | Optional? | Default    | Doc |
| :---- | :---- | :------ | :-------- | :--------- | :-- |
| `val` | `any` | _false_ | _false_   | _required_ | -   |

<a id="to_number"></a>

### `to_number(val: any)`

> Convert to number

#### Params

| Name  | Type  | VarArg? | Optional? | Default    | Doc |
| :---- | :---- | :------ | :-------- | :--------- | :-- |
| `val` | `any` | _false_ | _false_   | _required_ | -   |

<a id="assert"></a>

### `assert(cond: any, msg: string = Assertion failed)`

> Assertion

Flags: `@keywordish()`

#### Params

| Name   | Type     | VarArg? | Optional? | Default            | Doc |
| :----- | :------- | :------ | :-------- | :----------------- | :-- |
| `cond` | `any`    | _false_ | _false_   | _required_         | -   |
| `msg`  | `string` | _false_ | _true_    | `Assertion failed` | -   |

<a id="error"></a>

### `error(msg: string = Error)`

> Raise an error

Flags: `@returns(exit)`

#### Params

| Name  | Type     | VarArg? | Optional? | Default | Doc |
| :---- | :------- | :------ | :-------- | :------ | :-- |
| `msg` | `string` | _false_ | _true_    | `Error` | -   |

<a id="is_error"></a>

### `is_error(...val: any)`

> Check if it is an error

#### Params

| Name     | Type  | VarArg? | Optional? | Default | Doc |
| :------- | :---- | :------ | :-------- | :------ | :-- |
| `...val` | `any` | _true_  | _false_   | -       | -   |

<a id="unwrap"></a>

### `unwrap(...val: any) -> ...`

> Unwrap a result

#### Params

| Name     | Type  | VarArg? | Optional? | Default | Doc |
| :------- | :---- | :------ | :-------- | :------ | :-- |
| `...val` | `any` | _true_  | _false_   | -       | -   |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="expect"></a>

### `expect(val: any, msg: string = Got nil value) -> any`

> Expect a non-nil value

#### Params

| Name  | Type     | VarArg? | Optional? | Default         | Doc |
| :---- | :------- | :------ | :-------- | :-------------- | :-- |
| `val` | `any`    | _false_ | _false_   | _required_      | -   |
| `msg` | `string` | _false_ | _true_    | `Got nil value` | -   |

#### Returns

`any`<br/>

| Index | Type  |
| :---- | :---- |
| 0     | `any` |

<a id="get_metatable"></a>

### `get_metatable(val: table)`

> Get metatable

#### Params

| Name  | Type    | VarArg? | Optional? | Default    | Doc |
| :---- | :------ | :------ | :-------- | :--------- | :-- |
| `val` | `table` | _false_ | _false_   | _required_ | -   |

<a id="set_metatable"></a>

### `set_metatable(val: table, metatable: table | nil) -> table`

> Set metatable

#### Params

| Name        | Type    | VarArg? | Optional? | Default    | Doc        |
| :---------- | :------ | :------ | :-------- | :--------- | :--------- | --- |
| `val`       | `table` | _false_ | _false_   | _required_ | -          |
| `metatable` | `table  | nil`    | _false_   | _false_    | _required_ | -   |

#### Returns

`table`<br/>

| Index | Type    |
| :---- | :------ |
| 0     | `table` |

<a id="pairs"></a>

### `pairs(tab: table)`

> Return key-value iterator for table

Flags: `@returns(iterator)`

#### Params

| Name  | Type    | VarArg? | Optional? | Default    | Doc |
| :---- | :------ | :------ | :-------- | :--------- | :-- |
| `tab` | `table` | _false_ | _false_   | _required_ | -   |

<a id="ipairs"></a>

### `ipairs(tab: table)`

> Return index-value iterator for table

Flags: `@returns(iterator)`

#### Params

| Name  | Type    | VarArg? | Optional? | Default    | Doc |
| :---- | :------ | :------ | :-------- | :--------- | :-- |
| `tab` | `table` | _false_ | _false_   | _required_ | -   |

<a id="costatus"></a>

### `costatus(coroutine: any)`

> Get coroutine's status

#### Params

| Name        | Type  | VarArg? | Optional? | Default    | Doc |
| :---------- | :---- | :------ | :-------- | :--------- | :-- |
| `coroutine` | `any` | _false_ | _false_   | _required_ | -   |

<a id="try"></a>

### `try(func: function | table, ...params: any)`

> Run a function in protected mode, results follow Result Protocol

Flags: `@returns(result), @keywordish()`

#### Params

| Name        | Type      | VarArg? | Optional? | Default | Doc        |
| :---------- | :-------- | :------ | :-------- | :------ | :--------- | --- |
| `func`      | `function | table`  | _false_   | _false_ | _required_ | -   |
| `...params` | `any`     | _true_  | _false_   | -       | -          |

<a id="clone"></a>

### `clone(val: any) -> any`

> Clone a value (shallowly)

#### Params

| Name  | Type  | VarArg? | Optional? | Default    | Doc |
| :---- | :---- | :------ | :-------- | :--------- | :-- |
| `val` | `any` | _false_ | _false_   | _required_ | -   |

#### Returns

`any`<br/>

| Index | Type  |
| :---- | :---- |
| 0     | `any` |

<a id="curry"></a>

### `curry(f: function, ...args: any)`

> Bind arguments to a function partially

#### Params

| Name      | Type       | VarArg? | Optional? | Default    | Doc |
| :-------- | :--------- | :------ | :-------- | :--------- | :-- |
| `f`       | `function` | _false_ | _false_   | _required_ | -   |
| `...args` | `any`      | _true_  | _false_   | -          | -   |

<a id="result"></a>

### Static `Result`(DukaResult)

> Context for `result` protocol

[See here](#dukaresult)
