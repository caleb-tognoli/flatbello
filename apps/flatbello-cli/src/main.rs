//! Flatbello command-line interface: inspect, export, batch, test harness.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use flatbello_core::format::fixed;
use flatbello_core::{ExportOptions, FaceId, FaceKind, ImportOptions, sample};
use flatbello_engine::{Engine, ExtractOptions, ModelId, XAxisRule};

const USAGE: &str = "\
usage: flatbello-cli <command>

commands:
  faces <model>            list the faces of a STEP or IGES model
  export <model> --face <n> [--face <n> ...] [--format dxf|dxf-r12|svg]
         [--x-axis longest|plane] [--native-curves] [--closed-polylines] [--snap-holes]
         -o <file>
                           write the profiles of the given faces, side by side, in mm.
                           dxf is DXF R2000. --native-curves keeps ellipses and splines
                           (R2000 only); --closed-polylines writes each loop as one closed
                           LWPOLYLINE with arc bulges (R2000 only); --snap-holes rounds mesh
                           holes to standard drill sizes
  batch <model> --thickness <mm> [export options] -o <dir>
                           write every plate of that thickness (two flat faces facing
                           away from each other, that far apart) to its own file
  demo-export [-o <dir>]   write demo.dxf (R12) and demo.svg: a 100 x 60 mm plate with
                           5 mm corner radii and two round holes (default dir: .)
  help                     show this message";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match args.first().map(String::as_str) {
        Some("faces") => faces(rest),
        Some("export") => export(rest),
        Some("batch") => batch(rest),
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

/// Options shared by `export` and `batch`.
struct Job<'a> {
    model: &'a str,
    faces: Vec<FaceId>,
    thickness: Option<f64>,
    format: &'static str,
    out: &'a str,
    extract: ExtractOptions,
    export: ExportOptions,
}

fn parse_job<'a>(command: &str, args: &'a [String]) -> Result<Job<'a>, String> {
    let mut model: Option<&str> = None;
    let mut faces: Vec<FaceId> = Vec::new();
    let mut thickness = None;
    let mut format = "dxf-r2000";
    let mut out: Option<&str> = None;
    let mut extract = ExtractOptions::default();
    let mut export = ExportOptions::default();
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
            "--thickness" => {
                let v = value()?;
                let t: f64 = v.parse().map_err(|_| format!("bad thickness \"{v}\""))?;
                if !(t.is_finite() && t > 0.0) {
                    return Err(format!("bad thickness \"{v}\""));
                }
                thickness = Some(t);
            }
            "--format" => {
                format = match value()? {
                    "dxf" | "dxf-r2000" => "dxf-r2000",
                    "dxf-r12" => "dxf-r12",
                    "svg" => "svg",
                    other => return Err(format!("unknown format \"{other}\"")),
                }
            }
            "--x-axis" => {
                extract.x_axis = match value()? {
                    "longest" => XAxisRule::LongestStraightEdge,
                    "plane" => XAxisRule::PlaneX,
                    other => return Err(format!("unknown x-axis rule \"{other}\"")),
                }
            }
            "--native-curves" => export.native_curves = true,
            "--closed-polylines" => export.loops_as_polylines = true,
            "--snap-holes" => extract.snap_hole_sizes = true,
            "-o" => out = Some(value()?),
            _ if model.is_none() && !arg.starts_with('-') => model = Some(arg),
            _ => return Err(format!("unexpected argument \"{arg}\"\n\n{USAGE}")),
        }
    }
    let (Some(model), Some(out)) = (model, out) else {
        return Err(format!("{command} needs a model and -o <path>\n\n{USAGE}"));
    };
    Ok(Job {
        model,
        faces,
        thickness,
        format,
        out,
        extract,
        export,
    })
}

fn export(args: &[String]) -> Result<(), String> {
    let job = parse_job("export", args)?;
    if job.faces.is_empty() {
        return Err("export needs at least one --face".into());
    }
    let (engine, id) = open_model(job.model)?;
    let bytes = engine
        .export_faces(id, &job.faces, &job.extract, job.format, &job.export)
        .map_err(|e| e.to_string())?;
    write_file(Path::new(job.out), &bytes)
}

/// Writes every plate of the given thickness to its own file in the output folder.
fn batch(args: &[String]) -> Result<(), String> {
    let job = parse_job("batch", args)?;
    let thickness = job.thickness.ok_or("batch needs --thickness <mm>")?;
    let (engine, id) = open_model(job.model)?;
    let plates = engine
        .plates(id, thickness, &job.extract)
        .map_err(|e| e.to_string())?;
    if plates.is_empty() {
        return Err(format!("no {thickness} mm plates found in {}", job.model));
    }
    let dir = Path::new(job.out);
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let stem = Path::new(job.model)
        .file_stem()
        .map_or_else(|| "model".into(), |s| s.to_string_lossy().into_owned());
    let ext = if job.format == "svg" { "svg" } else { "dxf" };
    for (n, face) in plates.iter().enumerate() {
        let bytes = engine
            .export_faces(id, &[*face], &job.extract, job.format, &job.export)
            .map_err(|e| format!("plate {} (face {}): {e}", n + 1, face.0))?;
        let name = format!("{stem}_plate{}_face{}.{ext}", n + 1, face.0);
        write_file(&dir.join(name), &bytes)?;
    }
    println!("{} plate(s) of {thickness} mm", plates.len());
    Ok(())
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
