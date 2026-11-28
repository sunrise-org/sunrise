//! The parity harnesses' fixtures, run by the `sunrise-parity-fixtures` program (crates/sunrise-parity-fixtures, CI
//! only) with the same names and arguments as libtest, but without compiling the other tests. Each
//! gameplay crate discovers the `#[test]` wrappers and exposes their `run_<test>` functions behind
//! its `parity-fixtures` feature.

fn matches_filters(name: &str, filters: &[String], exact: bool) -> bool {
    filters.is_empty()
        || filters.iter().any(|filter| {
            if exact {
                name == filter
            } else {
                name.contains(filter.as_str())
            }
        })
}

/// Runs the fixtures the arguments select, one after another, each on a thread named for it as libtest names
/// its tests' threads; the exit code is libtest's (101 when one failed), and 101 too when the filters select none, so a
/// harness whose fixture left the table fails instead of passing on nothing.
#[cfg(feature = "parity-fixtures")]
pub fn main(fixtures: &[(&str, fn())]) {
    let mut filters = Vec::new();
    let (mut exact, mut list) = (false, false);
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--exact" => exact = true,
            "--list" => list = true,
            // Every fixture is an ignored test: these select it either way. Output is never captured.
            "--ignored" | "--include-ignored" | "--nocapture" | "--show-output" | "-q"
            | "--quiet" => {}
            "--test-threads" | "--color" | "--format" => {
                arguments.next();
            }
            other if other.starts_with("--test-threads=") || other.starts_with("--color=") => {}
            other if other.starts_with('-') => {
                eprintln!("sunrise-parity-fixtures: unsupported libtest option {other}");
                std::process::exit(101);
            }
            filter => filters.push(filter.to_owned()),
        }
    }
    let chosen: Vec<_> = fixtures
        .iter()
        .filter(|(name, _)| matches_filters(name, &filters, exact))
        .collect();
    if list {
        for (name, _) in &chosen {
            println!("{name}: test");
        }
        println!("\n{}, 0 benchmarks", tests(chosen.len()));
        return;
    }
    if chosen.is_empty() {
        eprintln!("sunrise-parity-fixtures: no fixture matches {filters:?}");
        std::process::exit(101);
    }
    let started = std::time::Instant::now();
    println!("\nrunning {}", tests(chosen.len()));
    let mut failed = Vec::new();
    for (name, fixture) in &chosen {
        let fixture = *fixture;
        let outcome = std::thread::Builder::new()
            .name((*name).to_owned())
            .spawn(fixture)
            .expect("starting a fixture's thread")
            .join();
        println!(
            "test {name} ... {}",
            if outcome.is_ok() { "ok" } else { "FAILED" }
        );
        if outcome.is_err() {
            failed.push(*name);
        }
    }
    if !failed.is_empty() {
        println!("\nfailures:");
        for name in &failed {
            println!("    {name}");
        }
    }
    println!(
        "\ntest result: {}. {} passed; {} failed; 0 ignored; 0 measured; {} filtered out; finished in {:.2}s\n",
        if failed.is_empty() { "ok" } else { "FAILED" },
        chosen.len() - failed.len(),
        failed.len(),
        fixtures.len() - chosen.len(),
        started.elapsed().as_secs_f64(),
    );
    if !failed.is_empty() {
        std::process::exit(101);
    }
}

/// "1 test", "2 tests": libtest's wording, which harnesses match.
#[cfg(feature = "parity-fixtures")]
fn tests(count: usize) -> String {
    format!("{count} test{}", if count == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::matches_filters;

    #[test]
    fn isolated_shell_fixture_matches_its_full_reexec_name() {
        let full = "cut_layout::game::client_side::crosshair_pick_tests::dynamic_redirectable_tag_controls_arrow_pick_and_in_ground_stops_it";
        let short = "game::client_side::crosshair_pick_tests::dynamic_redirectable_tag_controls_arrow_pick_and_in_ground_stops_it";
        assert!(matches_filters(full, &[full.into()], true));
        assert!(matches_filters(full, &[short.into()], false));
        assert!(!matches_filters(full, &[short.into()], true));
        assert!(!matches_filters(full, &["missing".into()], false));
    }
}
