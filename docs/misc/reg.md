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

---

# `unsafe`及内存相关

## ~~`repr`对齐~~ (不需要)

[See here](../../gc/src/header.rs)

为了保证GcHeader的存储排列方式不被编译器等改变
使用了`#[repr(C)]`

结构体的字段中最大的align是该结构体的align, 此处`align = 8`, 对于`info: u8` 需要再额外pad七个`u8`来使`type_id`对齐至`8`

## 裸指针

`size_of::<T>()`, `align_of::<T>()` 以此为大小和对齐信息

- `*mut T` 可变指针
- `*const T` 只读指针

- `*const ()` 不带类型的 仅是一个地址 (**0 size**, **1 align**)
- `*const u8` 指向一个字节(但是u8很特殊 可以当万能指针) (**1 byte size**, **1 align**)
