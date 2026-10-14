mod app;
mod mcp;
mod palette;
mod settings;
mod units;
mod view;

use clothes3d_core::{curves, dxf, model, sewing};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("clothes3d — pattern editor")
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "clothes3d",
        options,
        Box::new(|cc| {
            let path = std::env::args_os().nth(1).map(Into::into);
            Ok(Box::new(app::App::new(&cc.egui_ctx, cc.storage, path)))
        }),
    )
}
