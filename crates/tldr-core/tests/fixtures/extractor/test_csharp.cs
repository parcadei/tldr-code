// Expected: 0f 3c 7m (0 functions, 3 classes, 7 methods including constructors)
// Adversarial: C# has no free functions; all methods live inside classes

using System;

public class Animal
{
    public string Name { get; }

    public Animal(string name)
    {
        Name = name;
    }

    public virtual string Speak()
    {
        return "...";
    }

    public static Animal Create(string name)
    {
        return new Animal(name);
    }
}

public class Dog : Animal
{
    public Dog(string name) : base(name) { }

    public override string Speak()
    {
        return "Woof";
    }

    public string Fetch()
    {
        return "ball";
    }
}

public static class Utils
{
    public static int Add(int a, int b)
    {
        return a + b;
    }
}
