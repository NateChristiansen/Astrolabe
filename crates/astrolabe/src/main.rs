fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--version"] => println!("astrolabe {}", env!("CARGO_PKG_VERSION")),
        ["doctor" | "init"] => println!("not implemented (phase 1a)"),
        [] | ["--help" | "-h"] => println!("Usage: astrolabe <--version|doctor|init>"),
        _ => {
            eprintln!("Usage: astrolabe <--version|doctor|init>");
            std::process::exit(2);
        }
    }
}
