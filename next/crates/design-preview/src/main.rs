//! Disposable P2 design render. No real settings, storage, models or OS hooks.
use slint::{
    ComponentHandle,
    platform::{Platform, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::{path::PathBuf, rc::Rc, time::Instant};
slint::include_modules!();
mod review;

struct PreviewPlatform {
    window: Rc<MinimalSoftwareWindow>,
    start: Instant,
}
impl Platform for PreviewPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> std::time::Duration {
        self.start.elapsed()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|s| s == "review") {
        return review::render(&args[1..]);
    }
    if args.len() != 3 {
        return Err(
            "usage: kavychka-design-preview <overview|models|library> <light|dark> <output.png>"
                .into(),
        );
    }
    let page = match args[0].as_str() {
        "overview" => 0,
        "models" => 1,
        "library" => 2,
        _ => return Err("invalid view".into()),
    };
    let dark = match args[1].as_str() {
        "dark" => true,
        "light" => false,
        _ => return Err("invalid theme".into()),
    };
    let output = PathBuf::from(&args[2]);
    if output.exists() {
        return Err("output already exists; choose a new design file".into());
    }
    let window = MinimalSoftwareWindow::new(Default::default());
    slint::platform::set_platform(Box::new(PreviewPlatform {
        window: window.clone(),
        start: Instant::now(),
    }))?;
    let app = DesignWindow::new()?;
    app.global::<Palette>().set_dark(dark);
    app.set_page(page);
    app.show()?;
    window.set_size(slint::PhysicalSize::new(1100, 780));
    slint::platform::update_timers_and_animations();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1100, 780);
    if !window.draw_if_needed(|renderer| {
        renderer.render(pixels.make_mut_slice(), 1100);
    }) {
        return Err("preview was not rendered".into());
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image::save_buffer(output, pixels.as_bytes(), 1100, 780, image::ColorType::Rgb8)?;
    Ok(())
}
