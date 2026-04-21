// Expected: 4f 2c 0m (4 functions, 2 structs, 0 methods)
#include <stdio.h>

typedef struct {
    char name[64];
} Animal;

struct Dog {
    Animal base;
    char breed[32];
};

void animal_speak(Animal* a) {
    printf("%s\n", a->name);
}

int add(int x, int y) {
    return x + y;
}

static int helper(int x) {
    return x * 2;
}

int main(int argc, char** argv) {
    return 0;
}
