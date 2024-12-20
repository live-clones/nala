macro_rules! define_modules {
	($($module:ident),*) => {
		$(
			mod $module;
			pub use $module::$module;
		)*
	};
}

define_modules!(show, update, upgrade, install, history, fetch, clean);

mod list;
use anyhow::Result;
// TODO: These should maybe be part of like a libnala?
pub use history::{get_history, HistoryEntry, HistoryPackage};
use indexmap::IndexMap;
pub use list::list_packages;
use rust_apt::records::RecordField;
use rust_apt::{DepType, Version};
use serde::{Deserialize, Serialize};
use show::{format_local, show_dependency};
pub use upgrade::{apt_hook_with_pkgs, ask, run_scripts};

use crate::config::{color, Config, Theme};
use crate::util::URL;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Operation {
	Remove,
	AutoRemove,
	Purge,
	AutoPurge,
	Install,
	Reinstall,
	Upgrade,
	Downgrade,
	Held,
}

impl Operation {
	pub fn to_vec() -> Vec<Operation> {
		vec![
			Self::Remove,
			Self::AutoRemove,
			Self::Purge,
			Self::AutoPurge,
			Self::Install,
			Self::Reinstall,
			Self::Upgrade,
			Self::Downgrade,
		]
	}

	pub fn as_str(&self) -> &'static str {
		match self {
			Operation::Remove => "Remove",
			Operation::AutoRemove => "AutoRemove",
			Operation::Purge => "Purge",
			Operation::AutoPurge => "AutoPurge",
			Operation::Install => "Install",
			Operation::Reinstall => "ReInstall",
			Operation::Upgrade => "Upgrade",
			Operation::Downgrade => "Downgrade",
			Operation::Held => "Held",
		}
	}

	pub fn theme(&self) -> Theme {
		match self {
			Self::Remove | Self::AutoRemove | Self::Purge | Self::AutoPurge => Theme::Error,
			Self::Install | Self::Upgrade => Theme::Secondary,
			Self::Reinstall | Self::Downgrade | Self::Held => Theme::Notice,
		}
	}
}

impl std::fmt::Display for Operation {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.as_str())
	}
}

const RECORDS: [&str; 13] = [
	RecordField::Package,
	RecordField::Version,
	RecordField::Architecture,
	RecordField::Priority,
	RecordField::Essential,
	RecordField::Section,
	RecordField::Source,
	RecordField::InstalledSize,
	RecordField::Size,
	RecordField::Maintainer,
	RecordField::OriginalMaintainer,
	RecordField::Homepage,
	RecordField::SHA256,
];

fn print_info(header: &str, value: &str) {
	let sep = color::highlight!(":");
	let header = color::highlight!(header);
	println!("{header}{sep} {value}")
}

struct ShowVersion<'a> {
	ver: Version<'a>,
	records: IndexMap<&'static str, String>,
}

impl ShowVersion<'_> {
	pub fn new(ver: Version) -> ShowVersion {
		let records = IndexMap::from_iter(RECORDS.iter().copied().map(|key| {
			(
				key,
				ver.get_record(key).unwrap_or_else(|| "Unknown".to_string()),
			)
		}));
		ShowVersion { ver, records }
	}

	pub fn pretty_map(&self) -> IndexMap<&str, String> {
		let mut map = IndexMap::new();

		for (key, value) in &self.records {
			map.insert(*key, value.to_string());
		}

		for kind in DepType::iter() {
			let Some(deps) = self.ver.get_depends(kind) else {
				continue;
			};
			// These Dependency types will be colored red
			let red = if matches!(kind, DepType::Conflicts | DepType::DpkgBreaks) {
				Theme::Error
			} else {
				Theme::Primary
			};

			map.insert(
				kind.to_str(),
				show_dependency(deps, red).trim_end().to_string(),
			);
		}

		// Package File Section
		if let Some(pkg_file) = self.ver.package_files().next() {
			map.insert("Origin", pkg_file.origin().unwrap_or("Unknown").to_string());

			// Check if source is local, pacstall or from a repo
			let mut source = String::new();
			if let Some(archive) = pkg_file.archive() {
				if archive == "now" {
					source += &format_local(self.ver.parent().name());
				} else {
					let uri = self.ver.uris().next().unwrap();
					source += URL.find(&uri).unwrap().as_str();
					source += &format!(
						" {}/{} {} Packages",
						pkg_file.codename().unwrap(),
						pkg_file.component().unwrap(),
						pkg_file.arch().unwrap()
					);
				}
				map.insert("APT-Sources", source);
			}
		}

		// If there are provides then show them!
		// TODO: Add has_provides method to version
		// Package has it right now.
		let providers: Vec<String> = self
			.ver
			.provides()
			.map(|p| color::primary!(p.name()).into())
			.collect();

		if !providers.is_empty() {
			map.insert("Provides", providers.join(" "));
		}

		map.insert(
			"Description",
			self.ver
				.description()
				.unwrap_or_else(|| "Unknown".to_string()),
		);

		map
	}

	pub fn show(&self, config: &Config) -> Result<()> {
		if config.get_bool("machine", false) {
			println!("{}", self.to_json()?);
			return Ok(());
		}

		for (key, value) in &self.pretty_map() {
			print_info(key, value);
		}

		Ok(())
	}

	pub fn to_json(&self) -> Result<String> { Ok(serde_json::to_string_pretty(&self.ver)?) }
}
