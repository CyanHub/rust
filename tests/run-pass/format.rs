// compile-flags: -Zmiri-track-raw-pointers

fn main() {
    println!("Hello {}", 13);
    println!("{:0<width$}", "hello", width = 10);
}
