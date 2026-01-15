# Explanation of Interesting Thing I Thought

## Table of Content

-   NO!
-   I will just explan them crates by crates, partly

## `macro`

这里有大量的宏

### `binop`

用于声明二元运算符的优先级 方便动态更改 用于 Pratt 解析

### `errors`

低配版的`thiserror` 名为`ThatError`

### `history`

**史书云**的无意义`SemVer`宏 我自己都忘了用

### `info`

用得很多 专门为`enum`设计的 会有:

-   `name` -> `.name()`
-   `tag` -> `.is_xxx()`
-   `auto Display` 可以用`#[shy]`关闭
-   `#[idcard(type)]` -> `discrimination()` / `dicrimination2name()`

### `instructions`

很 DSL 很核心的一个东西 用来声明虚拟机指令 具体参见`backend`

### `visitors`

Visitor/VisitorMut 的宏 为了遍历 AST 做检查与解糖

## `shared`

### `ast`

我就在这里用`binop!`还有`Visitor`两个宏

### `utils`

这里有个`SemVer`但是并不完全 我还在完善它

### `constants`

这里放了很多常量 大多都是名字`str`这种的

### `builtin`

这个是用来当注册表的 比如给`builtin macro` 给`std`等等

### `error`

这里我用了`ThatError`宏 然后就是规定错误的

### `token`

这里大量使用了`Info`宏 规定了基本 token 们

### `types`

这就是一些`trait`之类的 让这个项目看起来更正式

### `value`

这里的 value 是`compile-time`的 也是可以序列化的(但是 table 我还没想好)
