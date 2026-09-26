//! Compare measured A/B JSON reports; a failure exits with a nonzero code.
use std::{env, fs, process::ExitCode};
use void_core::metrics::{BenchmarkEvidence, compare_runs};

fn main() -> ExitCode {
    match compare() {
        Ok(passed) => {
            if passed {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            }
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
fn compare() -> Result<bool, Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: cargo run -p void-core --example compare_metrics -- baseline.json candidate.json".into());
    }
    let baseline: BenchmarkEvidence = serde_json::from_slice(&fs::read(&args[0])?)?;
    let candidate: BenchmarkEvidence = serde_json::from_slice(&fs::read(&args[1])?)?;
    let acceptance = compare_runs(&baseline, &candidate);
    println!("{}", serde_json::to_string_pretty(&acceptance)?);
    Ok(acceptance.passed)
}
