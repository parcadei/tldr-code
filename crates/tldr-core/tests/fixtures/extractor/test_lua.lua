-- Expected: 5f 0c 0m (5 functions, 0 classes, 0 methods)
-- Adversarial: Lua has no native classes; table methods are still functions

local function topLevel(x)
    return x * 2
end

function globalFunc(x, y)
    return x + y
end

local anotherFunc = function(x)
    return x + 1
end

local Animal = {}
Animal.__index = Animal

function Animal.new(name)
    return setmetatable({name = name}, Animal)
end

function Animal:speak()
    return self.name
end
