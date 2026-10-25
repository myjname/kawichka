//! Batch renders of synthetic P2 states. Does not import any app/storage crate.
use crate::{PreviewPlatform, ReviewRow, ReviewWindow};
use image::ImageEncoder;
use serde::Deserialize;
use serde_json::json;
use slint::{
    ComponentHandle, ModelRc, VecModel, platform::software_renderer::MinimalSoftwareWindow,
};
use std::{path::PathBuf, time::Instant};

#[derive(Deserialize)]
struct Row {
    label: [String; 2],
    value: [String; 2],
    help: [String; 2],
    kind: String,
}
#[derive(Deserialize)]
struct Case {
    id: String,
    section: [String; 2],
    heading: [String; 2],
    subtitle: [String; 2],
    notice: [String; 2],
    notice_body: [String; 2],
    tone: String,
    primary: [String; 2],
    secondary: [String; 2],
    tertiary: [String; 2],
    sample: [String; 2],
    #[serde(default = "enabled")]
    primary_enabled: bool,
    rows: Vec<Row>,
    #[serde(default)]
    wizard: bool,
    #[serde(default)]
    companion: bool,
    #[serde(default = "none")]
    progress: f32,
}
fn none() -> f32 {
    -1.0
}
fn enabled() -> bool {
    true
}
#[derive(Deserialize)]
struct Viewport {
    id: String,
    width: u32,
    height: u32,
    scale: f32,
}
#[derive(Deserialize)]
struct Review {
    format: u32,
    cases: Vec<Case>,
    stress_cases: Vec<String>,
    viewports: Vec<Viewport>,
}
pub fn render(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err("review <review.json> <new-output-directory>".into());
    }
    let review: Review = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    if review.format != 1 || review.cases.is_empty() {
        return Err("invalid review fixture".into());
    }
    let output = PathBuf::from(&args[1]);
    if output.exists() {
        return Err("review output already exists; use a new directory".into());
    }
    std::fs::create_dir_all(&output)?;
    let window = MinimalSoftwareWindow::new(Default::default());
    slint::platform::set_platform(Box::new(PreviewPlatform {
        window: window.clone(),
        start: Instant::now(),
    }))?;
    let app = ReviewWindow::new()?;
    app.show()?;
    let mut results = Vec::new();
    for case in review.cases {
        if case.id.is_empty()
            || !case
                .id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err("invalid review ID".into());
        }
        for language in 0..2 {
            for dark in [false, true] {
                app.set_english(language == 1);
                app.set_dark(dark);
                app.set_section(case.section[language].as_str().into());
                app.set_heading(case.heading[language].as_str().into());
                app.set_subtitle(case.subtitle[language].as_str().into());
                app.set_notice(case.notice[language].as_str().into());
                app.set_notice_body(case.notice_body[language].as_str().into());
                app.set_tone(case.tone.as_str().into());
                app.set_primary(case.primary[language].as_str().into());
                app.set_secondary(case.secondary[language].as_str().into());
                app.set_tertiary(case.tertiary[language].as_str().into());
                app.set_sample(case.sample[language].as_str().into());
                app.set_primary_enabled(case.primary_enabled);
                app.set_wizard(case.wizard);
                app.set_companion(case.companion);
                app.set_progress(case.progress);
                app.set_rows(ModelRc::new(VecModel::from(
                    case.rows
                        .iter()
                        .map(|r| ReviewRow {
                            label: r.label[language].as_str().into(),
                            value: r.value[language].as_str().into(),
                            help: r.help[language].as_str().into(),
                            kind: r.kind.as_str().into(),
                        })
                        .collect::<Vec<_>>(),
                )));
                for (index, viewport) in review.viewports.iter().enumerate() {
                    if index > 0 && !review.stress_cases.contains(&case.id) {
                        continue;
                    }
                    let width = (viewport.width as f32 * viewport.scale).round() as u32;
                    let height = (viewport.height as f32 * viewport.scale).round() as u32;
                    app.window()
                        .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
                            scale_factor: viewport.scale,
                        });
                    window.set_size(slint::PhysicalSize::new(width, height));
                    for bottom in [false, true] {
                        if bottom
                            && index == 0
                            && app.get_scroll_overflow() < 1.0
                            && !review.stress_cases.contains(&case.id)
                        {
                            continue;
                        }
                        // Every form has top and bottom proof, including compact/high-DPI.
                        app.set_bottom(bottom);
                        app.set_focus_primary(bottom);
                        slint::platform::update_timers_and_animations();
                        app.window().request_redraw();
                        let mut pixels =
                            slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
                        if !window.draw_if_needed(|renderer| {
                            renderer.render(pixels.make_mut_slice(), width as usize);
                        }) {
                            return Err("review frame did not render".into());
                        }
                        if app.get_footer_clipped() || app.get_body_height() < 100.0 {
                            return Err(format!(
                                "layout overflow: {} / {} / {} / body {}",
                                case.id,
                                language,
                                viewport.id,
                                app.get_body_height()
                            )
                            .into());
                        }
                        let name = format!(
                            "{}-{}-{}-{}-{}.png",
                            case.id,
                            if language == 0 { "ru" } else { "en" },
                            if dark { "dark" } else { "light" },
                            viewport.id,
                            if bottom { "bottom" } else { "top" }
                        );
                        image::codecs::png::PngEncoder::new_with_quality(
                            std::fs::File::create(output.join(&name))?,
                            image::codecs::png::CompressionType::Fast,
                            image::codecs::png::FilterType::Sub,
                        )
                        .write_image(
                            pixels.as_bytes(),
                            width,
                            height,
                            image::ExtendedColorType::Rgb8,
                        )?;
                        results.push(json!({"case":case.id,"language":if language==0 {"ru"} else {"en"},"dark":dark,"viewport":viewport.id,"scale":viewport.scale,"width":width,"height":height,"bottom":bottom,"file":name,"body_height":app.get_body_height(),"scroll_overflow":app.get_scroll_overflow(),"footer_clipped":app.get_footer_clipped()}));
                    }
                }
            }
        }
    }
    app.invoke_focus_action();
    let count = app.get_activations();
    app.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "\n".into() });
    app.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "\n".into() });
    if app.get_activations() != count + 1 {
        return Err("keyboard activation failed".into());
    }
    std::fs::write(
        output.join("render-results.json"),
        serde_json::to_vec_pretty(
            &json!({"format":1,"renderer":"Slint 1.18.1 software","synthetic":true,"real_data_accessed":false,"keyboard_primary_enter":true,"windows_screen_reader_tested":false,"physical_dpi_monitor_tested":false,"frames":results}),
        )?,
    )?;
    println!(
        "Rendered {} P2 frames; keyboard Enter passed",
        results.len()
    );
    Ok(())
}
