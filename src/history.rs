use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct HistoryFile {
	entries: Vec<HistoryEntry>,
	version: String,
}

#[derive(Serialize, Deserialize)]
pub struct HistoryEntry {
	id: u32,
	date: String,
	requested_by: String,
	command: String,
	altered: u32,
	purged: bool,
	operation: Operation,
	explicit: Vec<String>,
	packages: Vec<HistoryPackage>,
}

#[derive(Serialize, Deserialize)]
pub struct HistoryPackage {
	name: String,
	version: String,
	old_version: Option<String>,
	size: u32,
	operation: Operation,
}

#[derive(Serialize, Deserialize)]
enum Operation {
	Remove,
	AutoRemove,
	Purge,
	AutoPurge,
	Install,
	Reinstall,
	Upgrade,
	Downgrade,
}

pub fn history_test() -> Result<()> {
	let pkg = HistoryPackage {
		name: "Nala".to_string(),
		version: "0.12.0".to_string(),
		old_version: None,
		size: 512,
		operation: Operation::Install,
	};

	let entry = HistoryEntry {
		id: 1,
		date: "2022-08-01 23:44:21 EDT".to_string(),
		requested_by: "root (0)".to_string(),
		command: "install nala".to_string(),
		altered: 12,
		purged: false,
		operation: Operation::Install,
		explicit: vec!["nala".to_string(), "another".to_string()],
		packages: vec![pkg],
	};

	let history = HistoryFile {
		entries: vec![entry],
		version: "0.2.0".to_string(),
	};

	let json = serde_json::to_string_pretty(&history)?;
	println!("{json}");
	Ok(())
}
