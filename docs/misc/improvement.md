# Improvement

## Bugs

- 注意副作用的函数
- `Less` `LessEqual`等指令逻辑混乱
- 寄存器分配问题: 注意lifetime, 防止需要连续寄存器的操作而占用仍存活的寄存器(`alloc_fresh`)
- GC finalizer内存释放问题: 接管所用权则有释放义务 否则仅借引用
- Assign等区分global & local, 保持统一配置
- GC finalizer存在类型混淆问题: 使用裸指针管理异构类型 必须存储类型信息 现GC存储TypeId

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
