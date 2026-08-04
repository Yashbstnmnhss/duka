# Standard Library for Duka

## Concepts

Duka is a 0-based language, which means indexes start from 0 instead of 1 (like Lua)

## Core

### Require

## Table

## String

- 字符串是**字节数组** `substr`/`slice`/`find` 的索引单位是字节,
  切在多字节字符中间会产生替换符
- 索引 **0-based** 负索引按尾部回绕(`-1` = 最后一个字节)
- `slice` 是左闭右开 `[start, end)`
- `substr` 的第三参数是长度 count
