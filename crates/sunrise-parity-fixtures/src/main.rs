use sunrise_gameplay_common::parity_fixture_cli as cli;

include!(concat!(env!("OUT_DIR"), "/parity_fixtures.rs"));

fn main() {
    mod_platform::__private::test_platform::install();
    cli::main(&FIXTURES);
}
