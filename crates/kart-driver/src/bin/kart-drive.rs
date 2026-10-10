//! `kart-drive <course.json> [--trace] [--class balanced|accel|speed|turn] [--time-limit s]`:
//! drives the course file and prints the report as JSON on stdout.

use std::process::ExitCode;

use kart_driver::{drive, Class, Course, DriveOpts};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut trace = false;
    let mut opts = DriveOpts::default();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--trace" => trace = true,
            "--class" => {
                opts.class = match args.next().as_deref() {
                    Some("accel") => Class::Accel,
                    Some("speed") => Class::Speed,
                    Some("turn") => Class::Turn,
                    _ => Class::Balanced,
                }
            }
            "--time-limit" => {
                if let Some(s) = args.next().and_then(|s| s.parse().ok()) {
                    opts.time_limit = s;
                }
            }
            _ => path = Some(a),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: kart-drive <course.json> [--trace] [--class c] [--time-limit s]");
        return ExitCode::from(2);
    };
    let course = match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| Course::from_json(&t))
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("kart-drive: {path}: {e}");
            return ExitCode::from(1);
        }
    };
    let report = drive(&course.world(), &course.route, &opts);
    println!("{}", report.to_json(trace));
    ExitCode::SUCCESS
}
