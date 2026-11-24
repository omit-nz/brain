//! The environment ll-core reads, set once before anything else runs.

use std::path::Path;

use brain::update::runtime_name;
use brain_platform::dirs::Dirs;

/// Points ll-core at brain's model folder and, when an install ships ONNX Runtime in `lib/` beside
/// the binary, at that library. Then lets ll-core resolve the runtime once, so every later call it
/// makes takes its warm path and sets the same value again. With no runtime set, nothing is
/// resolved: ll-core would download one, and brain never does.
pub fn prepare(dirs: &Dirs) -> anyhow::Result<()> {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .map(|dir| dir.join("lib").join(runtime_name()))
        .filter(|lib| lib.is_file());
    // SAFETY: `main` calls this first, before any thread exists, and `set_var` is unsound only
    // while another thread reads the environment. ll-core's `ensure_dylib` sets ORT_DYLIB_PATH on
    // every call; the plan's Handoff asks the ll-core owner for a release that sets it only when
    // it changes, which lets this workaround go.
    unsafe {
        std::env::set_var("LL_MODELS_DIR", &dirs.models);
        if let Some(lib) = &bundled {
            std::env::set_var("ORT_DYLIB_PATH", lib);
        }
    }
    if std::env::var_os("ORT_DYLIB_PATH").is_some_and(|p| Path::new(&p).is_file()) {
        ll_core::dylib::ensure_dylib()?;
    }
    Ok(())
}
