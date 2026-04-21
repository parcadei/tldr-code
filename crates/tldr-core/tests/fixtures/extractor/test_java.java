// Expected: 0f 3c 6m (0 functions, 3 classes, 6 methods)

public class Animal {
    private String name;

    public Animal(String name) {
        this.name = name;
    }

    public String speak() {
        return "...";
    }

    public static Animal create(String name) {
        return new Animal(name);
    }
}

class Dog extends Animal {
    public Dog(String name) {
        super(name);
    }

    public String fetch() {
        return "ball";
    }
}

class Utils {
    public static int add(int a, int b) {
        return a + b;
    }
}
