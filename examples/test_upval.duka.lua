local function counter()
    local n = 0
    return function()
        n = n + 1
        return n
    end
end

local c = counter()
print(c())
print(c())
print(c())

local function fact(n)
    if n <= 1 then
        return 1
    end
    return n * fact(n - 1)
end
print(fact(5))

local function pair()
    return 1, 2
end
local a, b = pair()
print(a, b)
