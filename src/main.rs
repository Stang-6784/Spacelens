#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod categories;
mod drives;
mod fmt;
mod icon;
mod kit;
mod mft;
mod scan;
mod theme;
mod tree;
mod treemap;
mod walk;
mod win;

use eframe::egui;
use std::sync::Arc;

fn main() -> eframe::Result {
    // Usage: Spacelens.exe [path-to-scan] [--snapshot out.png]
    let mut args = std::env::args().skip(1);
    let mut start = None;
    let mut snapshot = None;
    while let Some(a) = args.next() {
        if a == "--snapshot" {
            snapshot = args.next().map(std::path::PathBuf::from);
        } else if start.is_none() {
            start = Some(a);
        }
    }

    let icon = egui::IconData { rgba: icon::rgba(64), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Spacelens")
            .with_app_id("Spacelens")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1040.0, 640.0])
            .with_icon(Arc::new(icon)),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Spacelens",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, start, snapshot)))),
    )
}
