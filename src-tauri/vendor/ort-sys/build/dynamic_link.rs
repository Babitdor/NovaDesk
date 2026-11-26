#[cfg(feature = "copy-dylibs")]
use std::path::Path;

use crate::vars;

pub fn prefer_dynamic_linking() -> bool {
	match vars::get(vars::PREFER_DYNAMIC_LINK) {
		Some(val) => val == "1" || val.to_lowercase() == "true",
		None => false
	}
}

// Local patch (Nova Desktop). Upstream rc.12 assumed `OUT_DIR` sat at
// `<target>/<profile>/build/<package>-<hash>/out` and climbed three levels to
// reach the directory holding the executable. Current cargo nests it as
// `build/<package>/<hash>/out`, so that climb lands in `<target>/<profile>/build`
// and the copy step then panics — first on a missing `examples`/`deps`
// directory, then on the copy itself — which fails the whole build.
//
// The fix: find the `build` directory by name and use its parent, create the
// subdirectories instead of assuming them, and never fail the build over a
// side artifact: a missing DirectML.dll only matters to the DirectML execution
// provider, which this app does not use.
#[cfg(feature = "copy-dylibs")]
pub fn copy_dylibs(lib_dir: &Path, out_dir: &Path) {
	use std::fs;

	let Some(profile_dir) = out_dir
		.ancestors()
		.find(|dir| dir.file_name().is_some_and(|name| name == "build"))
		.and_then(Path::parent)
		.map(Path::to_path_buf)
	else {
		crate::log::warning!("could not find the target directory to place ONNX Runtime dylibs in");
		return;
	};

	for destination in [
		profile_dir.clone(),
		profile_dir.join("examples"),
		profile_dir.join("deps")
	] {
		if fs::create_dir_all(&destination).is_err() {
			continue;
		}
		let Ok(lib_files) = fs::read_dir(lib_dir) else {
			crate::log::warning!(
				"could not read `{}`; ONNX Runtime dylibs were not copied",
				lib_dir.display()
			);
			return;
		};
		for lib_file in lib_files.flatten() {
			let lib_path = lib_file.path();
			if lib_file.file_type().map(|kind| kind.is_dir()).unwrap_or(true) {
				continue;
			}
			let name = lib_path.to_string_lossy().to_lowercase();
			if ![".dll", ".so", ".dylib"].iter().any(|ext| name.ends_with(ext)) {
				continue;
			}
			let Some(lib_name) = lib_path.file_name() else {
				continue;
			};
			let out_path = destination.join(lib_name);
			if out_path.is_symlink() {
				let _ = fs::remove_file(&out_path);
			}
			if out_path.exists() {
				continue;
			}

			// A symlink keeps rebuilds cheap; Windows only allows one with
			// Developer Mode enabled, so fall back to a copy.
			#[cfg(windows)]
			let linked = std::os::windows::fs::symlink_file(&lib_path, &out_path).is_ok();
			#[cfg(not(windows))]
			let linked = std::os::unix::fs::symlink(&lib_path, &out_path).is_ok();
			if !linked && let Err(err) = fs::copy(&lib_path, &out_path) {
				crate::log::warning!("could not copy {}: {err}", lib_path.display());
			}
			if linked {
				println!("cargo:rerun-if-changed={}", out_path.display());
			}
		}
	}
}
