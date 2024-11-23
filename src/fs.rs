use std::path::Path;

use anyhow::{Context, Result};

/// This is just a Macro to deduplicate some code
///
/// Usage is a follows:
///
/// Single path parameter (like `read`, `read_to_string`, `create_dir_all`)
///
/// fs_with_context(Path, ReturnType)
///
/// Two parameters, typically `path` and `contents` (like `write`)
/// Or `path` and `path` (like `copy`)
///
/// fs_with_context(Path, Path/Contents, ReturnType)
macro_rules! fs_with_context {
	($name:ident, $return_type:ty) => {
		pub async fn $name<P: AsRef<Path> + std::fmt::Debug>(path: P) -> Result<$return_type> {
			tokio::fs::$name(&path)
				.await
				.with_context(|| format!("Failed to {} {:?}", stringify!($name), &path))
		}
	};

	($name:ident, $arg2_type:ty, $is_copy:expr, $return_type:ty) => {
		pub async fn $name<
			P: AsRef<Path> + std::fmt::Debug,
			C: AsRef<$arg2_type> + std::fmt::Debug,
		>(
			path: P,
			contents: C,
		) -> Result<$return_type> {
			tokio::fs::$name(&path, &contents).await.with_context(|| {
				if $is_copy {
					format!("copy from {:?} -> {:?}", path, contents)
				} else {
					format!("write {:?}", path)
				}
			})
		}
	};
}

pub async fn open_file<P: AsRef<Path> + std::fmt::Debug>(path: P) -> Result<tokio::fs::File> {
	tokio::fs::File::create(&path)
		.await
		.with_context(|| format!("Could not create file '{path:?}'"))
}

// Generate functions with the same names as std::fs counterparts
// fs_with_context!(read, Vec<u8>);
fs_with_context!(read_to_string, String);

fs_with_context!(remove_file, ());
fs_with_context!(create_dir_all, ());
fs_with_context!(remove_dir_all, ());

fs_with_context!(copy, Path, true, u64);
// fs_with_context!(write, [u8], false, ());
fs_with_context!(rename, Path, false, ());
