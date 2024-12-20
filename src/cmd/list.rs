use anyhow::Result;
use rust_apt::{Package, Version};

use crate::config::{color, Config};
use crate::debug;

/// List packages in a vector
///
/// Shared function between list and search
pub fn list_packages(config: &Config, packages: Vec<Package>) -> Result<()> {
	// We at least have one package so we can begin listing.
	let all_versions = config.get_bool("all_versions", false);
	let summary = config.get_bool("summary", false);
	let description = config.get_bool("description", false);

	for pkg in packages {
		if all_versions && pkg.has_versions() {
			for version in pkg.versions() {
				list_version(&version, summary, description)?;
			}
			continue;
		}

		// Get the candidate if we're only going to show one version.
		// Fall back to the first version in the list if there isn't a candidate.
		if let Some(version) = pkg.candidate().or(pkg.versions().next()) {
			// There is a version! Let's format it
			list_version(&version, summary, description)?;
			continue;
		}

		// There are no versions so it must be a virtual package
		let provides = pkg
			.provides()
			.map(|p| p.package().fullname(true))
			.collect::<Vec<_>>()
			.join(", ");

		println!(
			"{} (Virtual Pkg) [{provides}]",
			color::primary!(pkg.fullname(true))
		);
	}

	Ok(())
}

/// List a single version of a package
fn list_version(ver: &Version, summary: bool, description: bool) -> Result<()> {
	// Add the version to the string
	let pkg = ver.parent();
	let mut string = format!(
		"{} {}",
		color::primary!(&pkg.fullname(true)),
		color::ver!(ver.version()),
	);

	if let Some(pkg_file) = ver.package_files().next() {
		debug!("Package file found, building origin");

		let archive = pkg_file.archive().unwrap_or("Unknown");
		if archive != "now" {
			string += " [";
			string += pkg_file.origin().unwrap_or("Unknown");
			string += "/";
			string += pkg_file.codename().unwrap_or("Unknown");
			string += " ";
			string += pkg_file.component().unwrap_or("Unknown");
			string += "] ";
		}
	}

	// There is both an installed and candidate version
	if let (Some(installed), Some(candidate)) = (pkg.installed(), pkg.candidate()) {
		debug!(
			"Installed '{}' and Candidate '{}' exists, checking if upgradable",
			installed.version(),
			candidate.version(),
		);

		// Version is installed, check if it's upgradable
		if ver == &installed && ver < &candidate {
			string.push_str(&format!(
				" [Installed, Upgradable to: {}]",
				color::ver!(candidate.version())
			));
		}
		// Version isn't installed, see if it's the candidate
		if ver == &candidate && ver > &installed {
			string.push_str(&format!(
				" [Upgradable from: {}]",
				color::ver!(installed.version())
			));
		}
	}

	let mut attrs = vec![];
	// The version will not have an upgradable string, but is installed
	if ver.is_installed() {
		attrs.push("Installed");

		debug!("Version is installed and not upgradable.");
		// Version isn't downloadable, consider it locally installed
		if !ver.is_downloadable() {
			attrs.push("Local");
		}

		if pkg.is_auto_removable() {
			attrs.push("Auto-Removable");
		}

		if pkg.is_auto_installed() {
			attrs.push("Automatic");
		}
	}

	if !attrs.is_empty() {
		string.push('[');
		string.push_str(&attrs.join(", "));
		string.push(']');
	}

	if description {
		let desc = ver
			.description()
			.unwrap_or_else(|| "No Description".to_string());
		string += "\n";
		string += &desc;
	} else if summary {
		let desc = ver.summary().unwrap_or_else(|| "No Summary".to_string());
		string += "\n";
		string += &desc;
	}

	println!("{string}");
	Ok(())
}
