fn main() {
    eprintln!("The zylotek binary has moved to the zylotek-cli crate.");
    eprintln!("Build with: cargo build -p zylotek-cli");
    eprintln!("Or run with: cargo run -p zylotek-cli -- <command>");
    std::process::exit(1);
}
