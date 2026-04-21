use std::io::BufRead;

fn main() {
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf);
}
