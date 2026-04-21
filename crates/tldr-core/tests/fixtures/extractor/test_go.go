// Expected: 4f 2c 0m (4 functions, 2 structs, 0 methods -- Go methods are functions with receivers)
package main

func topLevel() int {
	return 42
}

func anotherFunc(x, y int) int {
	return x + y
}

type Animal struct {
	Name string
}

func (a *Animal) Speak() string {
	return a.Name
}

type Dog struct {
	Animal
	Breed string
}

func (d Dog) Fetch() string {
	return "ball"
}
