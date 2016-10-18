extern crate byteorder;

fn parse_args() -> String {
    let args: Vec<String> = ::std::env::args().collect();
    if args.len() == 2 {
        args[1].to_string()
    } else {
        println!("usage: {} <EXECUTABLE>", args[0]);
        ::std::process::exit(1);
    }

}

pub fn main() {
    let executable = parse_args();
    ::std::process::Command::new(executable);
}
