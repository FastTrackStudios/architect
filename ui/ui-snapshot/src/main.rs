//! ui-snapshot — CLI shim over the `ui_snapshot` library.
//!
//! Commands:
//!   ui-snapshot check        # render all scenes, fail on pixel diff
//!   ui-snapshot update       # regenerate + overwrite reference PNGs
//!   ui-snapshot render <name>  # render a single scene to target/ui-snapshots/

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use ui_snapshot::{SCENES, Scene, render_scene};

#[derive(Parser)]
#[command(name = "ui-snapshot")]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render all scenes and diff against committed references.
    Check {
        /// Per-scene fuzzy tolerance (0.0–1.0). Matches Blitz WPT runner default.
        #[arg(long, default_value_t = 0.1)]
        tolerance: f32,
    },
    /// Regenerate all committed reference PNGs.
    Update,
    /// Render a single scene to target/ui-snapshots/<name>.png.
    Render { name: String },
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Command::Check { tolerance } => check(tolerance),
        Command::Update => update(),
        Command::Render { name } => render_one(&name),
    }
}

fn write_png(path: &Path, buffer: &[u8], width: u32, height: u32) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create output dir");
    }
    let file = fs::File::create(path).expect("create png");
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut w = encoder.write_header().expect("png header");
    w.write_image_data(buffer).expect("png data");
    w.finish().expect("png finish");
}

fn reference_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/reference")
        .join(format!("{name}.png"))
}

fn output_path(name: &str) -> PathBuf {
    let ws = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    ws.join("target/ui-snapshots").join(format!("{name}.png"))
}

fn update() -> std::process::ExitCode {
    for scene in SCENES {
        println!(
            "render {} ({}×{})...",
            scene.name, scene.width, scene.height
        );
        let buf = render_scene(scene);
        let path = reference_path(scene.name);
        write_png(&path, &buf, scene.width, scene.height);
        println!("  wrote {}", path.display());
    }
    std::process::ExitCode::SUCCESS
}

fn render_one(name: &str) -> std::process::ExitCode {
    let Some(scene) = SCENES.iter().find(|s| s.name == name) else {
        eprintln!("no scene named {name}");
        return std::process::ExitCode::FAILURE;
    };
    let buf = render_scene(scene);
    let path = output_path(name);
    write_png(&path, &buf, scene.width, scene.height);
    println!("wrote {}", path.display());
    std::process::ExitCode::SUCCESS
}

fn check(tolerance: f32) -> std::process::ExitCode {
    let mut any_failed = false;
    for scene in SCENES {
        print!("check {} ... ", scene.name);
        std::io::stdout().flush().ok();

        let buf = render_scene(scene);
        let actual_path = output_path(scene.name);
        write_png(&actual_path, &buf, scene.width, scene.height);

        let reference_path = reference_path(scene.name);
        if !reference_path.exists() {
            println!("NO REFERENCE — run `ui-snapshot update` to create");
            any_failed = true;
            continue;
        }

        match diff_png(&actual_path, &reference_path, tolerance) {
            Ok(0) => println!("ok"),
            Ok(diff_count) => {
                println!("FAIL ({diff_count} differing pixels above tolerance {tolerance})");
                let diff_path = actual_path.with_file_name(format!("{}-diff.png", scene.name));
                println!("    actual:    {}", actual_path.display());
                println!("    reference: {}", reference_path.display());
                println!("    diff:      {}", diff_path.display());
                any_failed = true;
            }
            Err(e) => {
                println!("ERROR: {e}");
                any_failed = true;
            }
        }
    }
    if any_failed {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

/// Runs `dify::diff::run` between two PNGs. Writes a `<actual>-diff.png` if any
/// pixels exceed the tolerance.
fn diff_png(actual: &Path, reference: &Path, tolerance: f32) -> Result<usize, String> {
    let diff_path = actual.with_file_name(format!(
        "{}-diff.png",
        actual
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("scene")
    ));
    let left = actual.to_string_lossy().into_owned();
    let right = reference.to_string_lossy().into_owned();
    let out = diff_path.to_string_lossy().into_owned();
    let params = dify::diff::RunParams {
        left: &left,
        right: &right,
        output: &out,
        threshold: tolerance,
        output_image_base: Some(dify::cli::OutputImageBase::LeftImage),
        do_not_check_dimensions: false,
        detect_anti_aliased_pixels: true,
        blend_factor_of_unchanged_pixels: None,
        block_out_areas: None,
    };
    match dify::diff::run(&params) {
        Ok(Some(n)) if n > 0 => Ok(n as usize),
        Ok(_) => Ok(0),
        Err(e) => Err(format!("{e:?}")),
    }
}

// Silence dead-code warnings for the struct re-exported from the lib.
#[allow(dead_code)]
fn _unused(scene: &Scene) -> u32 {
    scene.width
}
