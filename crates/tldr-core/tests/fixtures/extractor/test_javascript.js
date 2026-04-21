// Expected: 5f 2c 4m (5 functions, 2 classes, 4 methods)
// Critical fixture: JS extractor was finding 0 functions vs 273 actual
function topLevel(x) {
    return x * 2;
}

const arrowFunc = (x) => x + 1;

const namedArrow = function helper(x) {
    return x * 3;
};

async function asyncFunc() {
    await fetch('/api');
}

function* generatorFunc() {
    yield 1;
    yield 2;
}

class Animal {
    constructor(name) {
        this.name = name;
    }

    speak() {
        return this.name;
    }

    static create(name) {
        return new Animal(name);
    }
}

class Dog extends Animal {
    fetch() {
        return "ball";
    }
}
