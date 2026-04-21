function greet(name: string): string {
    return name;
}
const result: number = greet("hello") as unknown as number;
