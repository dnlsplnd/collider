//! collider's desktop app: download HLS and MPEG-DASH streams and record live broadcasts.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
#[cfg(test)]
mod demo;
mod format;
mod history;
mod jobs;
mod settings;
#[cfg(test)]
mod tests;
mod theme;
mod views;
mod widgets;

use std::sync::Arc;

fn main() -> eframe::Result {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("collider-io")
        .build()
        .expect("cannot start the async runtime");
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("collider")
        .with_app_id("collider")
        .with_inner_size([1380.0, 880.0])
        .with_min_inner_size([960.0, 640.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")) {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let rt = runtime.handle().clone();
    eframe::run_native(
        "collider",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::App::new(
                &cc.egui_ctx,
                rt,
                app::Storage::user(),
            )))
        }),
    )
}
