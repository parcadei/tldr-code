// Expected: 3f 2c 4m (3 functions, 2 classes, 4 methods)
function topLevel(x: number): number {
    return x * 2;
}

const arrowFunc = (x: number): number => x + 1;

async function asyncFunc(): Promise<void> {
    await fetch('/api');
}

class Animal {
    constructor(public name: string) {}

    speak(): string {
        return this.name;
    }

    static create(name: string): Animal {
        return new Animal(name);
    }
}

class Dog extends Animal {
    fetch(): string {
        return "ball";
    }
}
