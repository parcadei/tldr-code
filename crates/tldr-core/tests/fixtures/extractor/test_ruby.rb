# Expected: 2f 2c 5m (2 functions, 2 classes, 5 methods)
# Adversarial: Ruby extractor was reporting 312 methods vs 67 actual (double-counting)
def top_level_func
  42
end

def another_func(x, y)
  x + y
end

class Animal
  attr_reader :name

  def initialize(name)
    @name = name
  end

  def speak
    "..."
  end

  def self.create(name)
    new(name)
  end
end

class Dog < Animal
  def speak
    "Woof"
  end

  def fetch
    "ball"
  end
end
