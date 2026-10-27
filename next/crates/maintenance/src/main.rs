#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use kavychka_maintenance::{Result, install, package};
use std::path::{Path, PathBuf};
fn launch(root: &Path, background: bool, verify: Option<&Path>) -> Result<()> {
    {
        let _guard = install::owned(root)?;
        install::recover(root)?;
    }
    let current = install::current(root)?
        .ok_or("Kavychka Next is not active; use the previous app or run setup")?;
    if !current.data.join("shell.sqlite3").is_file()
        || !std::fs::read(current.data.join("shell-sandbox.owner"))
            .is_ok_and(|b| b == b"kavychka-shell-sandbox-v1\n")
    {
        return Err("Installed data is unavailable; reconnect its disk or restore a profile. No empty library was created.".into());
    }
    let exe = package::app(root, &current.version)?;
    package::verify_installed(exe.parent().unwrap())?;
    let logs = current.data.join("logs");
    kavychka_storage::preview::no_link_chain(&logs)?;
    std::fs::create_dir_all(&logs)?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--data-dir")
        .arg(&current.data)
        .env("KAVYCHKA_INSTALL_ROOT", root)
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(logs.join("app.stderr.log"))?);
    if let Some(verify) = verify {
        cmd.arg("--library-ui-test").arg(verify);
    } else if background {
        cmd.arg("--background");
    }
    kavychka_maintenance::windows::hide(&mut cmd);
    if verify.is_some() {
        if !cmd.status()?.success() {
            return Err(
                "Installed application verification failed; see data/logs/app.stderr.log".into(),
            );
        }
    } else {
        cmd.spawn()?;
    }
    Ok(())
}
// The stable root executable only dispatches. Updated transaction/recovery code
// always runs from an immutable version directory, without replacing a running EXE.
fn delegate(root: &Path, args: &[std::ffi::OsString]) -> Result<()> {
    kavychka_maintenance::launch_context::ensure_paths(&[root])?;
    let uninstall = args.first().is_some_and(|s| s == "--uninstall");
    let mut version = if uninstall {
        Some(kavychka_maintenance::removal::current(root)?.version)
    } else {
        install::current(root)?.map(|c| c.version)
    };
    let journal = root.join("transaction.json");
    kavychka_storage::preview::no_link_chain(&journal)?;
    if journal.exists() {
        let j: serde_json::Value = serde_json::from_slice(&std::fs::read(journal)?)?;
        if j["format"] != 1 {
            return Err("Unsupported installation journal; run the matching setup".into());
        }
        if j["state"] != "complete" && j["state"] != "rolled_back" {
            version = j["candidate"]["version"].as_str().map(str::to_owned);
        }
    }
    let version = version.ok_or("Rust migration is inactive. Original app and new data are retained; use the old shortcut or run setup.")?;
    package::version(&version)?;
    let directory = root.join("versions").join(&version);
    package::verify_installed(&directory)?;
    let mut cmd = std::process::Command::new(directory.join("Kavychka.exe"));
    cmd.arg("--installed-root").arg(root).args(args);
    kavychka_maintenance::windows::hide(&mut cmd);
    if !cmd.status()?.success() {
        return Err("Kavychka could not start or recover. See launcher-error.log in the installation folder.".into());
    }
    Ok(())
}
fn run() -> Result<()> {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    let exe = std::env::current_exe()?;
    let mut root = exe
        .parent()
        .ok_or("missing installation root")?
        .to_path_buf();
    if args.first().is_some_and(|s| s == "--installed-root") {
        if args.len() < 2 {
            return Err("missing installed root".into());
        }
        root = PathBuf::from(&args[1]);
        args.drain(..2);
    } else if kavychka_maintenance::portable::is_portable(&root) {
        if args.iter().any(|a| a != "--background") {
            return Err("Portable.exe supports only --background".into());
        }
        let portable_root = root.clone();
        root = kavychka_maintenance::portable::prepare(&root, install::probe)?;
        let mut cmd = std::process::Command::new(root.join("Kavychka.exe"));
        cmd.args(&args).env("KAVYCHKA_PORTABLE_ROOT", portable_root);
        kavychka_maintenance::windows::hide(&mut cmd);
        cmd.spawn()?;
        return Ok(());
    } else if root.join("install.owner").is_file()
        && !args
            .first()
            .and_then(|s| s.to_str())
            .is_some_and(|s| matches!(s, "--pack" | "--apply"))
    {
        if std::fs::read(root.join("install.owner"))? != b"Kavychka.Next.v1\n" {
            return Err("unknown install owner".into());
        }
        return delegate(&root, &args);
    }
    let result = match args.first().and_then(|s| s.to_str()) {
        Some("--uninstall") if args.len() == 1 => {
            let c = kavychka_maintenance::removal::current(&root)?;
            let directory = root.join("versions").join(&c.version);
            package::verify_installed(&directory)?;
            let source = directory.join("app.exe");
            let scratch = std::env::temp_dir().join(format!(
                "Kavychka-remove-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_millis()
            ));
            kavychka_storage::preview::no_link_chain(&scratch)?;
            std::fs::create_dir(&scratch)?;
            let helper = scratch.join("Kavychka-Remove.exe");
            std::fs::copy(&source, &helper)?;
            if kavychka_maintenance::digest(&source)? != kavychka_maintenance::digest(&helper)? {
                return Err("Removal helper integrity check failed".into());
            }
            let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?;
            let mut cmd = std::process::Command::new(helper);
            cmd.arg("--remove-ui")
                .arg(&root)
                .arg(local)
                .env("KAVYCHKA_REMOVAL_RESUME", "0");
            kavychka_maintenance::windows::hide(&mut cmd);
            cmd.spawn()?;
            Ok(())
        }
        Some("--make-portable") if args.len() == 3 => {
            kavychka_maintenance::portable::create(Path::new(&args[1]), &exe, Path::new(&args[2]))
        }
        Some("--remove-preview") if args.len() == 3 => {
            let full = match args[1].to_str() {
                Some("program") => false,
                Some("all") => true,
                _ => return Err("Use program or all".into()),
            };
            let plan = kavychka_maintenance::removal::preview(&root, full)?;
            let output = Path::new(&args[2]);
            if output.exists() {
                return Err("Choose a new removal plan path".into());
            }
            kavychka_maintenance::atomic(output, &plan)
        }
        Some("--remove-apply") if args.len() == 3 => {
            let file = Path::new(&args[1]);
            kavychka_storage::preview::no_link_chain(file)?;
            if file.metadata()?.len() > 20_000_000 {
                return Err("Removal plan too large".into());
            }
            let plan: kavychka_maintenance::removal::Plan =
                serde_json::from_slice(&std::fs::read(file)?)?;
            kavychka_maintenance::removal::apply(
                &plan,
                Path::new(&args[2]),
                kavychka_maintenance::windows::unregister,
            )
        }
        Some("--verify-shortcuts") if args.len() == 2 => {
            kavychka_maintenance::windows::verify_shortcuts(Path::new(&args[1]))
        }
        Some("--pack") if args.len() == 5 => {
            let m = package::collect(Path::new(&args[2]), args[4].to_string_lossy().into())?;
            package::append(
                Path::new(&args[1]),
                Path::new(&args[2]),
                m,
                Path::new(&args[3]),
            )
        }
        Some("--apply") if args.len() == 4 => {
            install::apply(
                Path::new(&args[1]),
                Path::new(&args[2]),
                Path::new(&args[3]),
                install::probe,
            )?;
            Ok(())
        }
        Some("--verify-library") if args.len() == 2 => {
            launch(&root, false, Some(Path::new(&args[1])))
        }
        Some("--rollback") => {
            if install::rollback_with_platform(
                &root,
                kavychka_maintenance::windows::restore_legacy_startup,
            )?
            .is_some()
            {
                launch(&root, false, None)
            } else {
                eprintln!(
                    "Rust migration deactivated. Original app and the new generation are retained. Use the old shortcut."
                );
                Ok(())
            }
        }
        Some("--rollback-only") => {
            install::rollback(&root)?;
            Ok(())
        }
        Some("--background") => launch(&root, true, None),
        None => launch(&root, false, None),
        _ => Err("Unsupported launcher argument".into()),
    };
    if let Err(e) = &result
        && root.join("install.owner").is_file()
    {
        let _ = std::fs::write(root.join("launcher-error.log"), e.to_string());
    }
    result
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        #[cfg(windows)]
        if !std::env::args().nth(1).is_some_and(|a| {
            matches!(
                a.as_str(),
                "--pack"
                    | "--apply"
                    | "--installed-root"
                    | "--verify-library"
                    | "--rollback-only"
                    | "--verify-shortcuts"
            )
        }) {
            unsafe {
                let text: Vec<u16> = e.to_string().encode_utf16().chain(Some(0)).collect();
                let title: Vec<u16> = "Кавычка / Kavychka".encode_utf16().chain(Some(0)).collect();
                windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    std::ptr::null_mut(),
                    text.as_ptr(),
                    title.as_ptr(),
                    0x10,
                );
            }
        };
        std::process::exit(1)
    }
}
