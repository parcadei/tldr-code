// Expected: 2f 2c 4m (2 functions in object, 2 classes, 4 methods)
// Adversarial: Scala object defs vs class defs must be distinguished

object Utils {
  def topLevel(): Int = 42
  def anotherFunc(x: Int, y: Int): Int = x + y
}

class Animal(val name: String) {
  def speak(): String = "..."

  def describe(): String = s"Animal: $name"
}

class Dog(name: String) extends Animal(name) {
  override def speak(): String = "Woof"

  def fetch(): String = "ball"
}
