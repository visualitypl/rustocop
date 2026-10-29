//! Prints every registered cop name, one per line, for the Ruby tools.
use rustocop::cops::Registry;

fn main() {
    for cop in Registry::new().cops() {
        println!("{}", cop.name());
    }
}
