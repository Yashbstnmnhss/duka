local g = require("greeter")
print(g.hello)

local d = require("nested.deep")
print(d.value)

local i = require("initdir")
print(i.value)
