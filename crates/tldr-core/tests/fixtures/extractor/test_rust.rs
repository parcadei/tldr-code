// Expected: 3f 2c 4m (3 functions, 2 structs, 4 methods)

fn top_level() -> i32 {
    42
}

pub fn public_func(x: i32) -> i32 {
    x * 2
}

async fn async_func() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

struct Animal {
    name: String,
}

impl Animal {
    fn new(name: &str) -> Self {
        Animal {
            name: name.to_string(),
        }
    }

    fn speak(&self) -> &str {
        &self.name
    }
}

struct Dog {
    breed: String,
}

impl Dog {
    fn fetch(&self) -> &str {
        "ball"
    }

    fn bark(&self) -> &str {
        "woof"
    }
}
