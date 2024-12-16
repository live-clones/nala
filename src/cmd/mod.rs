macro_rules! define_modules {
	($($module:ident),*) => {
		$(
			mod $module;
			pub use $module::$module;
		)*
	};
}

define_modules!(show, update, upgrade, list, install, history, fetch, clean);
use anyhow::Result;
// TODO: These should maybe be part of like a libnala?
pub use history::{get_history, HistoryEntry, HistoryPackage};
pub use list::search;
use regex::{Regex, RegexBuilder};
use rust_apt::records::RecordField;
use rust_apt::{DepType, Package, Version};
use serde::{Deserialize, Serialize};
use show::{format_local, show_dependency};
pub use upgrade::{apt_hook_with_pkgs, ask, auto_remover, run_scripts};

use crate::config::{Config, Theme};

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

pub fn build_regex(pattern: &str) -> Result<Regex> {
	Ok(RegexBuilder::new(pattern).case_insensitive(true).build()?)
}

/// The show command
pub fn show_version<'a>(
	config: &Config,
	pkg: &'a Package,
	ver: &'a Version,
	pacstall_regex: &Regex,
	url_regex: &Regex,
) -> Vec<(&'static str, std::string::String)> {
	let mut version_map: Vec<(&str, String)> = vec![
		("Package", config.color(Theme::Primary, &pkg.fullname(true))),
		("Version", config.color(Theme::Secondary, ver.version())),
		("Architecture", ver.arch().to_string()),
		("Installed", ver.is_installed().to_string()),
		("Priority", ver.priority_str().unwrap_or("Unknown").into()),
		("Essential", pkg.is_essential().to_string()),
		("Section", ver.section().unwrap_or("Unknown").to_string()),
		("Source", ver.source_name().to_string()),
		("Installed-Size", config.unit_str(ver.installed_size())),
		("Download-Size", config.unit_str(ver.size())),
		(
			"Maintainer",
			ver.get_record(RecordField::Maintainer)
				.unwrap_or("Unknown".to_string()),
		),
		(
			"Original-Maintainer",
			ver.get_record(RecordField::OriginalMaintainer)
				.unwrap_or("Unknown".to_string()),
		),
		(
			"Homepage",
			ver.get_record(RecordField::Homepage)
				.unwrap_or("Unknown".to_string()),
		),
	];

	// Package File Section
	if let Some(pkg_file) = ver.package_files().next() {
		version_map.push(("Origin", pkg_file.origin().unwrap_or("Unknown").to_string()));

		// Check if source is local, pacstall or from a repo
		let mut source = String::new();
		if let Some(archive) = pkg_file.archive() {
			if archive == "now" {
				source += &format_local(pkg, config, pacstall_regex);
			} else {
				let uri = ver.uris().next().unwrap();
				source += url_regex.find(&uri).unwrap().as_str();
				source += &format!(
					" {}/{} {} Packages",
					pkg_file.codename().unwrap(),
					pkg_file.component().unwrap(),
					pkg_file.arch().unwrap()
				);
			}
			version_map.push(("APT-Sources", source));
		}
	}

	// If there are provides then show them!
	// TODO: Add has_provides method to version
	// Package has it right now.
	let providers: Vec<String> = ver
		.provides()
		.map(|p| config.color(Theme::Primary, p.name()))
		.collect();

	if !providers.is_empty() {
		version_map.push(("Provides", providers.join(" ")));
	}

	// TODO: Once we get down to the ol translations we need to figure out
	// If we will be able to use as_ref for the headers. Or get the translation
	// From libapt-pkg
	let dependencies = [
		("Depends", DepType::Depends),
		("Recommends", DepType::Recommends),
		("Suggests", DepType::Suggests),
		("Replaces", DepType::Replaces),
		("Conflicts", DepType::Conflicts),
		("Breaks", DepType::DpkgBreaks),
	];

	for (header, deptype) in dependencies {
		if let Some(depends) = ver.get_depends(&deptype) {
			// Dedupe dependencies as they have duplicates sometimes
			// Believed to be due to multi arch
			let mut depend_names = vec![];
			let mut deduped_depends = vec![];

			for dep in depends {
				let name = dep.first().name();
				if !depend_names.contains(&name) {
					depend_names.push(name);
					deduped_depends.push(dep);
				}
			}

			// These Dependency types will be colored red
			let red = if matches!(deptype, DepType::Conflicts | DepType::DpkgBreaks) {
				Theme::Error
			} else {
				Theme::Primary
			};

			version_map.push((
				header,
				show_dependency(config, &deduped_depends, red)
					.trim_end()
					.to_string(),
			));
		}
	}

	version_map.push((
		"Description",
		ver.description().unwrap_or_else(|| "Unknown".to_string()) + "\n",
	));

	version_map
}
