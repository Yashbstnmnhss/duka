---
title: Attributes
description: '@declare, @inline, @const, ... in duka'
---

Attributes can be attached at **function**, **variables** and **object**. Some of them accept properties. Either `key = ...` or `key(...)` is ok. Only **constant values** are supported (like string, number, boolean and nil).

To pass multiple values, you can wrap them with `(` `)`: `@a(b(1, 2, 3))` passes `[1, 2, 3]` to property `b` in attribute `a`. Identifiers in values turn into its literal string: `@a(b = abc)` makes `b = "abc"`.

See [here](../../reference/attributes) for available attributes in duka.
