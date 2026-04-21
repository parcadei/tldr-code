# Expected: 3f 2c 5m (3 functions, 2 classes, 5 methods)

def top_level_func():
    pass

def another_func(x, y):
    return x + y

async def async_func():
    await something()

class Animal:
    def __init__(self, name):
        self.name = name

    def speak(self):
        return "..."

    @staticmethod
    def create(name):
        return Animal(name)

class Dog(Animal):
    def speak(self):
        return "Woof"

    @classmethod
    def from_shelter(cls, id):
        return cls(f"dog-{id}")
