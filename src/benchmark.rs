extern crate byteorder;
extern crate clap;

pub fn main() {
    use clap::{App, Arg};
    let matches = App::new("Zillions benchmarker")
        .version("0.0.0")
        .about("Does awesome things")
        .arg(Arg::with_name("EXECUTABLE")
             .required(true)
             .index(1)
             .help("The executable to benchmark"))
        .get_matches();

    let executable = matches.value_of("EXECUTABLE").unwrap();

    println!("exectuable: {}", executable);

    let addr = "127.0.0.1:8080";
    let child = ::std::process::Command::new(executable)
        .arg(addr)
        .spawn();

}
