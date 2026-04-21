<?php
// Expected: 2f 2c 5m (2 functions, 2 classes, 5 methods)
// Adversarial: PHP extractor was returning empty (missing extractor)

function topLevelFunc(): int {
    return 42;
}

function anotherFunc(int $x, int $y): int {
    return $x + $y;
}

class Animal {
    public function __construct(
        private string $name
    ) {}

    public function speak(): string {
        return "...";
    }

    public static function create(string $name): self {
        return new self($name);
    }
}

class Dog extends Animal {
    public function speak(): string {
        return "Woof";
    }

    public function fetch(): string {
        return "ball";
    }
}
