# Logic Programming

See [WAM](../../backend/src/vm/logic.rs)

```
fact color(red)
fact color(blue)
```

Query:

```
color(X)
```

Generated:

```js
Constants:
0: "red"
1: "blue"
```

```js
Instructions:
0: UnifyVar(0)      // Put X into register 0
1: Call(color)      // Call `color` fact
2: Try(5)           // Insert a choice point, jumping to pc=5 if failed
3: UnifyConst(0, 0) // Let X(R0) = Constant 0 ("red")
4: Succeed          // Branch succeed, return single result or back to choice point to get more results
5: UnifyConst(0, 1) // Let X(R0) = Constant 1 ("blue")
6: Succeed          // Branch succeed, choice points are empty, program done
```
