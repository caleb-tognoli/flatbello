//! Flatbello command-line interface: inspect, export, batch, test harness.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use flatbello_core::{ExportOptions, sample};

const USAGE: &str = "\
usage: flatbello-cli <command>

commands:
  demo-export [-o <dir>]   write demo.dxf (R12) and demo.svg: a 100 x 60 mm plate with
                           5 mm corner radii and two round holes (default dir: .)
  help                     show this message";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("demo-export") => demo_export(&args[1..]),
        Some("help" | "-h" | "--help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command \"{other}\"\n\n{USAGE}")),
        None => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

fn demo_export(args: &[String]) -> Result<(), String> {
    let dir = match args {
        [] => PathBuf::from("."),
        [flag, dir] if flag == "-o" => PathBuf::from(dir),
        _ => return Err(format!("bad arguments for demo-export\n\n{USAGE}")),
    };
    let profile = sample::rounded_plate().map_err(|e| format!("building demo profile: {e}"))?;
    let options = ExportOptions::default();
    for (format, file) in [("dxf-r12", "demo.dxf"), ("svg", "demo.svg")] {
        let bytes = flatbello_engine::export(std::slice::from_ref(&profile), format, &options)
            .map_err(|e| e.to_string())?;
        write_file(&dir.join(file), &bytes)?;
    }
    Ok(())
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}
