// Expected: 2f 2c 5m (2 functions, 2 classes, 5 methods)
// Adversarial: Kotlin was finding 8 vs 12 actual (under-counting)
fun topLevel(): Int {
    return 42
}

fun anotherFunc(x: Int, y: Int): Int = x + y

class Animal(val name: String) {
    fun speak(): String {
        return "..."
    }

    companion object {
        fun create(name: String): Animal = Animal(name)
    }
}

class Dog(name: String) : Animal(name) {
    fun speak(): String = "Woof"

    fun fetch(): String {
        return "ball"
    }

    fun describe(): String = "Dog: $name"
}
