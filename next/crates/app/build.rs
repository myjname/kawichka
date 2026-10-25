fn main() {
    println!("cargo:rerun-if-env-changed=KAVYCHKA_RELEASE_VERSION");
    let version = std::env::var("KAVYCHKA_RELEASE_VERSION")
        .unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").unwrap());
    let parts: Vec<_> = version.split('.').collect();
    assert!(
        parts.len() == 3
            && parts.iter().all(|s| !s.is_empty()
                && s.len() <= 8
                && s.bytes().all(|c| c.is_ascii_digit())
                && (s.len() == 1 || !s.starts_with('0'))),
        "Invalid release version"
    );
    println!("cargo:rustc-env=KAVYCHKA_RELEASE_VERSION={version}");
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .with_style("fluent".into())
            .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles),
    )
    .unwrap();
}
