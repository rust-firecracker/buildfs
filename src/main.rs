use std::path::PathBuf;

use schema::Config;

mod schema;

fn main() {
    let path = PathBuf::from("/home/kanpov/Documents/debian-min.toml");
    let content = std::fs::read_to_string(path).unwrap();
    dbg!(toml::from_str::<Config>(&content).unwrap());
}
