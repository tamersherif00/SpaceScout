#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod drives;
mod fmt;
mod scan;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Headless benchmark: spacescout.exe --bench <path>
    if args.first().map(String::as_str) == Some("--bench") {
        let rest = args[1..].join(" ");
        let path = std::path::PathBuf::from(if rest.is_empty() { "C:\\".to_string() } else { rest });
        let progress = scan::Progress::default();
        let t = scan::run(path, &progress);
        println!(
            "files={} dirs={} size={} errors={} nodes={} time={:.2}s",
            t.nodes[0].files,
            progress.dirs.load(std::sync::atomic::Ordering::Relaxed),
            fmt::bytes(t.nodes[0].size),
            t.errors,
            t.nodes.len(),
            t.elapsed.as_secs_f64()
        );
        for &i in t.top_files.iter().take(5) {
            println!("  {:>10}  {}", fmt::bytes(t.nodes[i as usize].size), t.path_of(i).display());
        }
        return Ok(());
    }

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SpaceScout")
            .with_inner_size([1150.0, 760.0])
            .with_min_inner_size([700.0, 400.0]),
        ..Default::default()
    };
    // Join so an unquoted path with spaces ("C:\Program Files") still works.
    let start = (!args.is_empty()).then(|| args.join(" "));
    eframe::run_native(
        "SpaceScout",
        opts,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, start)))),
    )
}
