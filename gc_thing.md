# なんぜ GC がある？

> GC がとても重要な物

You know what, Rust doesn't use GC to manage memory.
Rust gives us **RC** things, which refers to _Reference Counting_.
But it has some apparent weaknesses, for example: cyclic data, which, in lua, may easily be created.

# GC を実現することが難しいか？

> はい， あまり簡単ではありません

Garbage collector needs to identify all objects whether are accessible in heap, in that way, GC can manage memory completely, meanwhile drop garbage punctually.

But the question lies in, if there exist pointers that have nothing else pointing to them, then how can we trace them? That also means, once we use GC in our program, we create a sea of objects, where require us to manage them carefully, or it would go troublesome (unsafety)

Same, in the aspect of Rust language, implementing a GC brings some `unsafe` code to our project. Certainly, not every `unsafe` code is bad, but me, as a Rust learner, should pay more attention into them.

# 分かた，さあ、始めましょう！

## Allocate

Each GCObject must be created by allocator

## Roots

Through these roots, allocator is able to trace all of reachable objects. By marking the reachables, it can sweep garbage(objects that are not marked)

## Collecting

-   标记&清除 (Mark&Sweep)
-   分代 GC (Generational)
-   增量 GC (Incremental)
