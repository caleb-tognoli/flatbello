//! Flatbello command-line interface: inspect, export, batch, test harness.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use flatbello_core::format::fixed;
use flatbello_core::{ExportOptions, FaceId, FaceKind, ImportOptions, Tolerance, sample};
use flatbello_engine::{Engine, ExtractOptions, ModelId, XAxisRule};

const USAGE: &str = "\
usage: flatbello-cli <command>

commands:
  faces <model>            list the faces of a STEP or IGES model
  export <model> --face <n> [--face <n> ...] [--format dxf|svg] [--x-axis longest|plane]
         -o <file>         write the profiles of the given faces, side by side
                           (default format: dxf, which is DXF R12 in millimetres)
  demo-export [-o <dir>]   write demo.dxf (R12) and demo.svg: a 100 x 60 mm plate with
                           5 mm corner radii and two round holes (default dir: .)
  help                     show this message";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match args.first().map(String::as_str) {
        Some("faces") => faces(rest),
        Some("export") => export(rest),
        Some("demo-export") => demo_export(rest),
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

fn open_model(path: &str) -> Result<(Engine, ModelId), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading {path}: {e}"))?;
    let mut engine = Engine::new();
    let id = engine
        .open(&bytes, path, &ImportOptions::default())
        .map_err(|e| format!("{path}: {e}"))?;
    Ok((engine, id))
}

fn faces(args: &[String]) -> Result<(), String> {
    let [path] = args else {
        return Err(format!("faces takes one model file\n\n{USAGE}"));
    };
    let (engine, id) = open_model(path)?;
    let model = engine.model(id).map_err(|e| e.to_string())?;
    println!("{:>5}  {:<11}  {:>12}  normal", "face", "kind", "area mm²");
    for f in &model.faces {
        let kind = match f.kind {
            FaceKind::Planar => "planar",
            FaceKind::Cylindrical => "cylindrical",
            FaceKind::Other => "other",
        };
        let normal = model
            .face_normal(f.id)
            .map(|n| format!("({}, {}, {})", fixed(n.x, 3), fixed(n.y, 3), fixed(n.z, 3)))
            .unwrap_or_default();
        println!("{:>5}  {:<11}  {:>12.3}  {normal}", f.id.0, kind, f.area);
    }
    Ok(())
}

fn export(args: &[String]) -> Result<(), String> {
    let mut model: Option<&str> = None;
    let mut faces: Vec<FaceId> = Vec::new();
    let mut format = "dxf-r12";
    let mut out: Option<&str> = None;
    let mut x_axis = XAxisRule::LongestStraightEdge;
    let mut it = args.iter().map(String::as_str);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg {
            "--face" => {
                let v = value()?;
                faces.push(FaceId(
                    v.parse().map_err(|_| format!("bad face number \"{v}\""))?,
                ));
            }
            "--format" => {
                format = match value()? {
                    "dxf" | "dxf-r12" => "dxf-r12",
                    "svg" => "svg",
                    other => return Err(format!("unknown format \"{other}\"")),
                }
            }
            "--x-axis" => {
                x_axis = match value()? {
                    "longest" => XAxisRule::LongestStraightEdge,
                    "plane" => XAxisRule::PlaneX,
                    other => return Err(format!("unknown x-axis rule \"{other}\"")),
                }
            }
            "-o" => out = Some(value()?),
            _ if model.is_none() && !arg.starts_with('-') => model = Some(arg),
            _ => return Err(format!("unexpected argument \"{arg}\"\n\n{USAGE}")),
        }
    }
    let (Some(model), Some(out)) = (model, out) else {
        return Err(format!("export needs a model and -o <file>\n\n{USAGE}"));
    };
    if faces.is_empty() {
        return Err("export needs at least one --face".into());
    }

    let (engine, id) = open_model(model)?;
    let tolerance = Tolerance::default();
    let bytes = engine
        .export_faces(
            id,
            &faces,
            &ExtractOptions { tolerance, x_axis },
            format,
            &ExportOptions { tolerance },
        )
        .map_err(|e| e.to_string())?;
    write_file(Path::new(out), &bytes)
}

fn demo_export(args: &[String]) -> Result<(), String> {
    let dir = match args {
        [] => PathBuf::from("."),
        [flag, dir] if flag == "-o" => PathBuf::from(dir),
        _ => return Err(format!("bad arguments for demo-export\n\n{USAGE}")),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
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
