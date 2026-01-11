use btclib::types::Block;
use btclib::util::Savable;
use std::env;
use std::fs::File;

fn main() {
    let path = if let Some(arg) = env::args().nth(1) {
        arg
    } else {
        eprintln!("Usage: block_print <block_file>");
        std::process::exit(1);
    };

    if let Ok(file) = File::open(path) {
        let block = Block::load(file).expect("Failed to load block");
        println!("{:#?}", block);
    }
}
