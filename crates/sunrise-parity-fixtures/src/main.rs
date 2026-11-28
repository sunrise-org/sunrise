use sunrise_gameplay_common::parity_fixture_cli as cli;

include!(concat!(env!("OUT_DIR"), "/parity_fixtures.rs"));

fn main() {
    cli::install_test_platform();
    cli::main(&FIXTURES);
}
