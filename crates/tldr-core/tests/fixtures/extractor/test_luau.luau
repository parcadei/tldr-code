-- Expected: 3f 0c 0m (3 functions, 0 classes, 0 methods)
-- Adversarial: Luau typed Lua has no native classes; type aliases are not classes

local function topLevel(x: number): number
    return x * 2
end

local function anotherFunc(x: number, y: number): number
    return x + y
end

type Animal = {
    name: string,
    speak: (self: Animal) -> string,
}

local function createAnimal(name: string): Animal
    local animal: Animal = {
        name = name,
        speak = function(self: Animal): string
            return self.name
        end,
    }
    return animal
end
