// Expected: 2f 2c 5m (2 functions, 2 classes, 5 methods)
#include <string>

int globalFunc(int x) {
    return x * 2;
}

namespace utils {
    int helperFunc(int a, int b) {
        return a + b;
    }
}

class Animal {
public:
    Animal(const std::string& name) : name_(name) {}
    virtual ~Animal() = default;
    virtual std::string speak() const { return "..."; }

private:
    std::string name_;
};

class Dog : public Animal {
public:
    Dog(const std::string& name) : Animal(name) {}
    std::string speak() const override { return "Woof"; }
    std::string fetch() const { return "ball"; }
};
