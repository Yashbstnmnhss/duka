# Improvement

记录一些Bug还有性能提升

## Fixed Bugs

- 注意副作用的函数
- `Less` `LessEqual`等指令逻辑混乱
- 寄存器分配问题: 注意lifetime, 防止需要连续寄存器的操作而占用仍存活的寄存器(`alloc_fresh`)
- GC finalizer内存释放问题: 接管所用权则有释放义务 否则仅借引用
- Assign等区分global & local, 保持统一配置
- GC finalizer存在类型混淆问题: 使用裸指针管理异构类型 必须存储类型信息 现GC存储TypeId

## Open Bugs

- GC只标记一层: `Tracer::mark` 把直接子对象染灰并push进`gray_list` 但`collect_with_finalizer`从不drain这个队列
  深度>=2的可达对象仍是White 会被sweep释放 已复现 STATUS_HEAP_CORRUPTION
- GC颜色从不重置: `set_color(White)`只在`GcHeader::init`出现 幸存对象永远停在Gray
  第一次collect之后再也不会释放任何东西 GC直接失效
- 触发点在`VM::collect_if_need`(`backend/src/vm/mod.rs:525`) 阈值默认256(`Heap::threshold`)

## 一

### 常量预物化

常量在先前的代码中一直是用一次就在常量池中**clone**一次(转化为RuntimeValue) 造成严重浪费

目前在DukaProto里添加了常量缓存 将在init时一次性转化ConstValue为RuntimeValue
此后均由GC处理, 不再重新分配 (见`gcstress` bench)

### Concat优化

删除了一下几点:

1. `vec![]`反复扩容
2. 多余的中间`ConstValue`转化
3. 多余的`ConstValue`转化为`RuntimeValue`

目前直接预分配总长, 并直接构造`RuntimeValue` (见`strcat` bench)
