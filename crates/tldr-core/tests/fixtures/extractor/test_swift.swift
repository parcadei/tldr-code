// Expected: 3f 2c 5m (3 functions, 2 classes, 5 methods)
// Adversarial: Swift extractor was returning empty (missing extractor)
func topLevel() -> Int {
    return 42
}

func anotherFunc(_ x: Int, _ y: Int) -> Int {
    return x + y
}

func asyncFunc() async throws -> Data {
    return try await URLSession.shared.data(from: url).0
}

class Animal {
    let name: String

    init(name: String) {
        self.name = name
    }

    func speak() -> String {
        return "..."
    }

    static func create(name: String) -> Animal {
        return Animal(name: name)
    }
}

class Dog: Animal {
    override func speak() -> String {
        return "Woof"
    }

    func fetch() -> String {
        return "ball"
    }
}
