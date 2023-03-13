use std::fs::{read_dir, remove_file};
use crate::config::Config;

fn remove_files(le_file: &str) {
	if let Ok(paths) = read_dir(le_file) {
		for path in paths {
			let file = path.unwrap();
			if file.file_type().unwrap().is_file() {
				remove_file(file.path()).unwrap();
			}
		}
	}

}

pub fn clean(config: &Config) {
	if config.get_bool("lists", false) {
		remove_files("/var/lib/apt/lists/");
		remove_files("/var/lib/apt/lists/partial/");
		return;
	}
	if config.get_bool("fetch", false) {
		remove_files("/etc/apt/sources.list.d/nala-sources.list");
		return;
	}
	remove_files("/var/cache/apt/archives/");
	remove_files("/var/cache/apt/lists/partial/");
	return;
}
