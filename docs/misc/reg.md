# 寄存器分配

即时寄存器分配(on-the-fly)

---

See [shared/ir.rs](../../shared/src/ir.rs) _Allocator_

### `alloc()`

- 局部 表达式结果所用
- 优先free_list最小的寄存器, 否则分配top

### `alloc_fresh()`

- call frame
- 无视free_list, 直接分配top

### `alloc_temp()`

- 一次性 存放中间临时结果
- 分配top

### `alloc_consecutive_*()`

- 多返回`TakeMany` `TakeAll`使用
- 必须保证分配区域连续

### `free()` / `free_many()`

- 结束寄存器生命周期
- 返还free_list

---

See [frontend/ir.rs](../../frontend/src/ir.rs) _IRGenerator_

### `is_local_reg()`

局部作用域绑定的寄存器 保证其存活

### `recycle_anonymous_from()`

每个**statement**之后调用 用于回收所有死寄存器
(寄存器存活 = `is_local_reg() == true`)
