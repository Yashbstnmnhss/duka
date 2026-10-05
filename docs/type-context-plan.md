# type-context 企划(施工备忘)

状态:**设计定稿,未开工**。基线 `cargo test --workspace` 680 全绿,`cargo fmt --check` 干净。

本文记录 2026-10-01 一整轮讨论的结论。下次施工从「P0 第一个测试」开始,见文末。

---

## 0. 语言表面(定稿)

```duka
function translate<T, U>(shape: T, offset: U): V
where
    U: Point,           -- A 显式约束:挂到 binder
    Sized(T),           -- B 隐式约束:谓词,bool 语义
    type V = T          -- C 内部绑定:右侧是任意 type expr
end
```

配套规则:

- `<T: B = D>` 表面不变,内部降级成 `<T = D> where T: B`。`=` 是默认值,调用方可覆盖。
- `type U = E` 是**定义**,`U = E` 是**断言相等**(预留)。
- `type function` / `type fn` 可以当类型实参传入;`where U(T)` 是「应用 + bool 语义」。
- pack:单个 named,`<Ts...>` 声明,`x: Ts...` 使用。
- **不加**:type function 的签名类型、kind 标记、变型规则、多 pack。
- **总规则**:只在 type-context 存在的东西,一旦流到 value-context 就退化或报错。

### 与 value-context 的对应(追求一致正交)

| value-context | type-context |
|---|---|
| `local U = T` | `type U = T` |
| `U = T` | `U = T`(断言) |
| `x: T` | `U: B` |
| `local {a,b} = e` | `type {A,B} = T` |
| `function f<T>(x) end` | `where U(T)` |

---

## 1. 执行顺序(核心,第一版写错过)

**`<>` 只声明变量,where 块是求解阶段,求完才有具体类型,然后才检查 param / return / body。**

where 块是一个纯函数,同一份代码跑两遍:

```rust
enum WhereMode { Generic, Monomorphic }   // 声明期 / 每个调用点

fn run_where(
    constraints: &[WhereClause],
    vars: &HashMap<Box<str>, Type>,      // Generic 全是 Type::Param;Monomorphic 已是具体 Type
    mode: WhereMode,
) -> (Vec<(Box<str>, Type)>,            // binds:U -> V
      Vec<Obligation>,                  // 泛型模式下未能兑现的
      Vec<DukaSpannedError>)
```

### 声明期(Generic)

| 步 | 做什么 |
|---|---|
| ① | `<T, U>` 声明变量:`T -> Type::Param("T")`, `U -> Type::Param("U")` |
| ② | `run_where(Generic)`:`U: Point` 记 obligation 并把 bound 挂到 solver 的 `VarDecl`;`Sized(T)` 记 obligation;`type V = T` 得到 `V -> Type::Param("T")`(符号传播) |
| ③ | 全部泛型 = `T`、`U`(仍是 Param)、`V`(= `T` 的符号别名) |
| ④ | 检查 param(`shape: T`, `offset: U`)、return(`V`)、body |

### 调用点(Monomorphic)

| 步 | 做什么 |
|---|---|
| ① | 由实参解出等式:`T := int`、`U := Point` |
| ② | `run_where(Monomorphic)`:`Point.accepts(Point)` ✓;`Sized(int)` → `Literal(Bool(true))` ✓;`type V = T` → `V := int` |
| ③ | 全部泛型具体:`T=int`、`U=Point`、`V=int` |
| ④ | 检查实参匹配、return(`int`)、body 的单态视图(LSP 用) |

### 实现要点(踩过的坑)

- **A 的 bound 求值必须放 checker 的 `visit_func_block`**(`typechecker.rs:835-864`)。ScopeAnalyzer 跑的时候类型环境还不存在(`U: array<T>` 里的 `T`、`Require(...)`、模块类型都不可用),这正是现在 bound 存渲染字符串 `Option<Box<str>>` 的原因。`VarDecl.bound`(`solver.rs:12-21`)改成 `Option<Arc<Type>>`,`SymbolType::TypeParam.bound`(`utils.rs:351`)同步。
- **`solver.rs:137-144` 的 `substitute` 不递归代入**。`type V = T` 让 `V` 的绑定是 `Param("T")`,单查会停在 `Param("T")`。binds 必须**按声明顺序边代边存**(与 `solver.rs:124-132` 处理 `default` 的现成做法一致),或把 substitute 改成迭代到不动点。倾向前者。
- **`V` 在返回标注里的解析顺序**:`: V` 在 where 之前出现,而 `V` 由 where 定义。`StmtKind::Function` 分支(`typechecker.rs:712-738`)要改成:声明类型参数 → 跑 `run_where(Generic)` → 把 binds 的名字追加进 `normalize_generic_names`(`:475-580`)的名单 → 再建签名。
- **where 块内严格自上而下**:`U: Point` 的约束要在 `Sized(T)` 之前可见,不重排。
- **前向引用允许符号传播**:`where type V = T, T: array<V>` 里 `V` 和 `T` 都留在 Param 域,单态模式兑现。
- **谓词不进 solver**。solver 文档明确「no notion of functions, scopes or modules」(`solver.rs:1-5`),`Pred`/`Assert` 的求值在 typechecker 的单态模式里做。
- **A 约束价值双份**:单态模式决定 `U` 的具体类型;泛型模式给 body 提供成员表。类型参数在 `bind_params`(`eval.rs:1164-1192`)里要绑成 **bound 而不是 `Any`**,这样 `offset.x` 在 body 里才有依据。

---

## 2. 数据结构

```rust
// ast.rs —— 用死掉的 WhereClause stub(ast.rs:712-718)换真 payload
TypeParam(Name, ParamKind, Option<TypeDesc> /*default*/>)     // ParamKind { Plain, Pack }
pub struct GenericDecl { params: Box<[TypeParam]>, constraints: Box<WhereClause> }
pub enum WhereClause {
    Bound(Name, Box<Expr>),          // A
    Pred(Box<Expr>),                 // B / Sized(T)
    Bind(Destructing, Box<Expr>),    // C,复用 ast.rs:269-280
    Assert(Box<Expr>, Box<Expr>),
}
enum Obligation { Bound{..}, Pred{..}, Assert{..} }
enum ParamShape { Fixed(TypeDesc), Pack(Box<str>) }
struct GenericSig<'a> { params, shapes: Vec<ParamShape>, constraints }
Solver::constrain_call(&mut self, sig: &GenericSig, formals, actuals, span)   // 现 solver.rs:82
```

改动点清单:

| 位置 | 现状 | 目标 |
|---|---|---|
| `ast.rs:189-197` `FuncBody.1` | `Box<[TypeParam]>` | `Box<GenericDecl>`(一处覆盖 function / type function / type fn) |
| `ast.rs:348` `ObjectDef.type_params` | `Box<[TypeParam]>` | `Box<GenericDecl>` |
| `ast.rs:166-174` `InlineTypeFunction` | `(Name, Box<[Param]>, Box<TypeDesc>)` | 加第 4 个字段装 `GenericDecl` |
| `ast.rs:722-758` `TypeDesc` | 17 变体 | 加 `Expr(Box<Expr>)` 逃逸变体 |
| `utils.rs:351-354` `SymbolType::TypeParam` | `bound/default: Option<Box<str>>`(渲染字符串) | `bound: Option<Arc<Type>>` + `kind` |
| `utils.rs` `SymbolType` | 无 | 新增 `TypeLocal`(类型级 `local`) |
| `solver.rs:12-21` `VarDecl.bound` | `Option<Type>` | 已是 `Option<Type>`,补 obligation 出口 |
| `solver.rs:65-134` `Solver` | `new(vars)` / `constrain_call(&[Type], ...)` | 输入换成 `GenericSig` |
| `solver.rs:90-134` `solve` | `given: Vec<Type>` | 增 obligation 兑现阶段 |
| `dtype.rs:15-50` `Type` | 17 变体 | 加 `TypeFn { id, name }` |
| `analyzer/mod.rs` `ScopeAnalysis` | 有 `call_cache` | 加 `closures: Arc<Mutex<Vec<TypeClosure>>>` |

`FuncBody` 加字段的破坏面:数到约 8 处解构(`typechecker.rs:428/715/838`、`analyzer/mod.rs:538`、`eval.rs:1164/1237`、`ir.rs`、`modules.rs`),开工时以编译器为准。

---

## 3. type function 当类型值

现状:`TypeValue::Closure` 只能活在 `EvalCtx` 内。`TypeValue::to_type()`(`tyval.rs:60-65`)遇 Closure 映射成 `Type::Any`,而它在每道边界都被调:`typechecker.rs:348`(`resolve_type`)、`eval.rs:508`、`eval.rs:622`。显式类型实参 `f.<T>(...)` 走的正是 `typechecker.rs:1531` 的 `ty_args.iter().map(resolve_type)` —— 闭包在那里被抹掉。

`docs/type.md:146` 已经宣称支持 HKT,但 `lib/tests/type_fn.rs:698` 那个唯一的 `type fn` 测试只断言「能编译」,闭包从没被应用过。

### 做法:intern 一个 id

`shared` 只依赖 `duka-macros`,**不能持有 `FuncBody`**(它在 frontend),所以闭包进 `Type` 只能带 id。仓库现成同构模式:

| 现有 | id 指向 |
|---|---|
| `Type::Object { id, name, .. }`(`dtype.rs:20`) | `ScopeAnalysis.objects` |
| `SymbolType::TypeFunction(usize)`(`utils.rs:347`) | `ScopeAnalysis.type_fns` |
| `TypeValue::Tagged { ty, id }`(`tyval.rs:12`) | `ScopeAnalysis.call_cache` |

照抄:`Type::TypeFn { id: TypeFnId, name: Box<str> }`。

- intern 表挂 **`ModuleBuildCache` 级别**而非 per-chunk。已有的 `Tagged.id → call_cache` 是 per-chunk 且被 module 缓存共享,存在同类悬垂 id 的老问题,新表不重复这个错。
- `to_type()` 遇 Closure → intern → `Type::TypeFn`;反向 `from_type` 在 eval 解析名字时取回闭包,让 `apply_closure`(`eval.rs:956`)仍是唯一应用路径。
- `typechecker.rs:1531` 改走新的 `resolve_type_value(&TypeDesc) -> TypeValue`,不经过 `to_type()`。
- **solver 一行不用改**:`bindings: HashMap<Box<str>, Type>`(`solver.rs:57`)装得下,`substitute` 的 `Type::Param(name) => bindings.get(name)` 天然把 `U` 代成 `Type::TypeFn`。
- 3 处**穷尽** match 必须改:`dtype.rs:134` Display(打闭包名)/ `dtype.rs:328` accepts(同 id 同一性,不做子类型)/ `visitors.rs:901` `type_to_checker`。
- 收紧 `tyval.rs:43` 的 `Closure => true` 洞(否则闭包实参能满足任何形参)。

### 已有的高阶支持(不用重造)

`bind_params`(`eval.rs:1164-1192`)把类型级实参以 `TypeValue` 存进 frame;`apply_closure`(`eval.rs:956`)能应用;`eval_expr_to_type` 的 `Expr` 调用分支(`eval.rs:2094`)已处理「callee 是 frame 里的闭包」;`TypeDesc::TypeCall`(`eval.rs:679-686`)是唯一把实参求值成 `TypeValue` 并保留闭包的构造。所以

```duka
type function Apply(fn, t) return fn(t) end
Apply(type fn(x) x?, int)
```

**很可能今天就是通的**(待 P0 第一个测试验证)。`builtin.duka` 里那 10 个类型函数只是没用上,不是不支持。

### 多态性

靠 **body + 调用点类型环境**(`TypeClosure.captured` 帧 + `bind_params`),**不靠声明处的 `forall`**。所以不需要签名类型、不需要变型。

---

## 4. 阶段

### P0 type expr

- `ExprKind` 加 `#[tag(tcx)]` —— `Info` 宏零改动,`#[tag(ident)]` 生成 `is_<ident>`(`macros/src/info.rs:44, 81-90`)。注意已有 `is_typesys` / `is_lit` 全仓库零调用,生成了不等于有人管,闸门要显式调。
- **单一递归闸门** `type_level_ok(&Expr)`,白名单来自 `is_tcx()`。parser 早拒绝 / eval 入口校验 / LSP 补全三处共用,否则三处会漂移。
- `TypeDesc::Expr` 变体 + `parse_type_expr` / `resolve_type_expr`(`resolve_type` 降为封装,现有 40 处调用点零改动)。
- 内联求值发真错误:替掉 `eval.rs:793/804/817/821` 的静默 `Type::Any`,启用一直没被用过的 `UnknownType`(`errors.rs:189`)。
- `type(expr)` 保持独立(`eval.rs:648` 现在直接返回 `Any`),加「暂不支持」诊断。**不接**,接了就是 checker 重入。
- `SymbolType::TypeLocal` + LSP hover / 补全。
- `finish_member`(`mod.rs:2482-2556`)补 `.<` 分支,与 value 侧 `PathSuffix::TypeArgs`(`mod.rs:1582-1610` / `:1691-1715`)对称。
- **where 解析必须用 `then(TokenKind::Where)`**,因为 `must_keyword`(`mod.rs:2964`)只认 `Ident`。

**P0 第一个测试(验证性假设)**:

```rust
type function Apply(fn, t) return fn(t) end
Apply(type fn(x) x?, int)
```

判断已通,但**没验证过**。通 → P1 只是把闭包暴露出去;不通 → 先补 `eval.rs:2094` 一带。

### P1 type function 当类型值

第 3 节全部。独立可测。

### P2 where clause

- `GenericDecl` 全套 + parser 四处接入(`mod.rs:947` object / `:1778` / `:1880` / `:641` 顺带修被丢弃的 `<T>`)。
- **`ty_def`(`mod.rs:591-666`)现在用手写扫描找配平的 `<...>` 判 inline/块体,加了 where 会误判**,要改成调 `opt_ty_par_def_list`。
- `StmtKind::Function` 分支改成「声明参数 → `run_where(Generic)` → normalize(带 bind 名) → 建签名」。
- `var_decls`(`typechecker.rs:1367`)→ `GenericSig`;`generic_fns`(`:168`,现在只装 `Box<[TypeParam]>`)换成能存约束的结构。
- `bind_params` 把类型参数绑成 bound 而不是 `Any`。
- `report_solution`(`:1453`)加 obligation 失败映射;`record_bindings`(`:1419`)把约束带给 LSP。
- object 的 `type_params` 现在从没被 checker 读过,一并接上。

### P3 pack(单个 named)

- `PACK_PARAM_PREFIX` marker 照抄 `REC_PARAM_PREFIX`(`dtype.rs:53-64`),Display 打回 `Ts...`。
- 放开 `ty_par_def_list` 里对 `...` 的显式拒绝(`mod.rs:1848`,现在报 "expected generic")。
- `x: Ts...` = `ExprKind::VarArg(TypeLit(Ts))` —— 节点已存在(`mod.rs:2027`),不需要新 `TypeDesc` 变体。pack 解到 `Type::TypeTuple`(它本来就是「一串类型」)。
- `ParamShape::Pack` 驱动 splat。
- 类型级内建:`Ts[0]` / `Len` / `++` / `First` / `Rest`。
- value 侧 `...` 匿名语义不动(`Param::Var` 无名、VM `frame.var_args` 不动)。

### P4(默认不做)

多 pack `<A..., B...>` + shape unification(Rust `SameShape` 那套)· `<` 比较回落 · `type(expr)`。

---

## 5. 测试计划

- `lib/tests/type_fn.rs`:高阶类型函数(`Apply`)· 闭包当类型实参 · 闭包泄漏到 value-context 报错 · 内联 type expr(算术/比较/条件)· `UnknownType` 诊断。
- `lib/tests/generics.rs`:A 命中/违反 · B 真假 · C 跨参数可见(`y: V` 引用 where 里定义的 `V`)· 前向引用符号传播 · `<T: int = int>` 可被覆盖 · 未绑定诊断。
- pack:`First<Ts...>` · pack 绑定 · value 侧 `...` 行为不变。
- LSP:where 子句 hover · 类型级局部 hover / 补全 · bounded param 的成员补全 · `.{pack}` 补全。
- **回归必看** `frontend/src/lib.rs:92` 的 `transformer_test`(linq `where`)。`where` 变硬关键字那次就是它报的警 —— `linq_clause` 原来用 `then_keyword("where")` 按文本匹配,改成 `then(TokenKind::Where)` 才对。
- 每阶段 `cargo test --workspace` + `cargo fmt --check`。

---

## 6. 明确不做

- `type(expr)` 的 checker 重入(闭包捕获运行时值 → 值类型 → 类型求值 → 再回 checker,checker 有状态:`types` 栈 / `ret_stack` / `collect_mode` / 两趟返回推导)。
- type function 的签名类型、kind 标记、变型规则。
- 多 pack 与 shape unification。
- `<` 比较回落 —— 现在 `mod.rs:2321-2351` 在类型位置无回溯地当泛型应用(`Ident <` 一律泛型,注释自己标了是尖角),所以 `x: a < b` 今天就是解析错误。要比较用 `Lt(a, b)` 内建。
- destructure 的 pack 切片(`(Head, ...Rest)`),等 P4。只覆盖已有类型投影:`Bind(name)` 和 `Term(Array/Table)` 的字段 / 下标(`TypeDesc::Access` / `Type::TypeTable` / `type_access_inner` `eval.rs:602`)。
- value-context `...` 的联动推断。

---

## 7. 开工前要验证的三点

1. **高阶类型函数今天是否已通**(`Apply` 那个测试)。这是 P1 的地基。
2. **`FuncBody` 加字段的破坏面**:约 8 处解构,以编译器为准。
3. **`Type::TypeFn` 的 id 跨分析生命周期**:`Arc<Type>` 被 `span_mapper` 或 module 缓存复用时,id 可能在另一个 `ScopeAnalysis` 里指错。挂 session 级表能避免,需确认 `ModuleBuildCache` 的构造点能否承载。

---

## 附:本轮修完的东西(label / goto,与本企划无关但同批改动)

label 与 goto 统一走 `SymbolTable`,没有第二份真值来源:

- `Symbols.gotos: Vec<(Box<str>, Span)>` 放在 `labels` 旁边(`shared/src/utils.rs:404` 附近)。
- 查询:`goto_label_at` / `goto_target` / `label_at` / `label_uses` / `goto_targets`。
- `StmtKind::Label/Goto` 改成 `Name`(带 span),记录名字本身的位置而不是整条语句,和 `symbol_at_span` 同一套。改了 `ast.rs:68-69`、`parser/mod.rs:510-514/524-528`、`analyzer/mod.rs:341-362`、`analyzer/visitors.rs:212-217`、`ir.rs:1201-1208`、`transpiler.rs:346-347`。
- LSP:goto definition、label references、label/goto hover、goto 的 `-> line N` inlay hint。
- 前向 goto 能解析(表查名字,不依赖遍历顺序);label 不跨函数(checker 同时报 `InvisibleGotoLabel`)。

新增测试 3 个:前向 goto、无目标 goto、label 不跨函数。
