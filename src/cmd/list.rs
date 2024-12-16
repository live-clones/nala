use anyhow::Result;
use rust_apt::{new_cache, Package, Version};

use crate::config::{Config, Theme};
use crate::{dprint, glob};

/// The search command
pub fn search(config: &Config) -> Result<()> {
	let mut out = std::io::stdout().lock();
	let cache = new_cache!()?;
	list_packages(
		config,
		glob::regex_pkgs(config, &cache)?.only_pkgs(),
		&mut out,
	)?;

	Ok(())
}

/// The list command
pub fn list(config: &Config) -> Result<()> {
	let mut out = std::io::stdout().lock();
	let cache = new_cache!()?;

	// List the packages that were found
	list_packages(
		config,
		if config.pkg_names().is_ok() {
			glob::pkgs_with_modifiers(config, &cache)?.only_pkgs()
		} else {
			cache.packages(&glob::get_sorter(config)).collect()
		},
		&mut out,
	)?;

	Ok(())
}

/// List packages in a vector
///
/// Shared function between list and search
fn list_packages(
	config: &Config,
	packages: Vec<Package>,
	out: &mut impl std::io::Write,
) -> Result<()> {
	// We at least have one package so we can begin listing.
	for pkg in packages {
		if config.get_bool("all_versions", false) && pkg.has_versions() {
			for version in pkg.versions() {
				write!(
					out,
					"{} ",
					config.color(Theme::Primary, &pkg.fullname(true))
				)?;
				list_version(out, config, &pkg, &version)?;
				list_description(out, config, &version)?;
			}
			// The new line is a little weird if we print descriptions
			if !config.get_bool("description", false) && !config.get_bool("summary", false) {
				// New line to separate package groups
				writeln!(out)?;
			}
			continue;
		}

		// Write the package name
		write!(
			out,
			"{} ",
			config.color(Theme::Primary, &pkg.fullname(true))
		)?;

		// Get the candidate if we're only going to show one version.
		// Fall back to the first version in the list if there isn't a candidate.
		if let Some(version) = pkg.candidate().or(pkg.versions().next()) {
			// There is a version! Let's format it
			list_version(out, config, &pkg, &version)?;
			list_description(out, config, &version)?;
			continue;
		}

		// There are no versions so it must be a virtual package
		list_virtual(out, config, &pkg)?;
	}

	Ok(())
}

/// List a single version of a package
fn list_version<'a>(
	out: &mut impl std::io::Write,
	config: &Config,
	pkg: &Package<'a>,
	version: &Version<'a>,
) -> std::io::Result<()> {
	// Add the version to the string
	dprint!(
		config,
		"list_version for {} {}",
		pkg.name(),
		version.version()
	);

	write!(out, "{}", config.color_ver(version.version()))?;

	if let Some(pkg_file) = version.package_files().next() {
		dprint!(config, "Package file found, building origin");

		let archive = pkg_file.archive().unwrap_or("Unknown");

		if archive != "now" {
			let origin = pkg_file.origin().unwrap_or("Unknown");
			let component = pkg_file.component().unwrap_or("Unknown");
			write!(out, " [{origin}/{archive} {component}]")?;
		}
	}

	// There is both an installed and candidate version
	if let (Some(installed), Some(candidate)) = (pkg.installed(), pkg.candidate()) {
		dprint!(
			config,
			"Installed '{}' and Candidate '{}' exists, checking if upgradable",
			installed.version(),
			candidate.version(),
		);

		// Version is installed, check if it's upgradable
		if version == &installed && version < &candidate {
			return writeln!(
				out,
				" [Installed, Upgradable to: {}]",
				config.color_ver(candidate.version()),
			);
		}
		// Version isn't installed, see if it's the candidate
		if version == &candidate && version > &installed {
			return writeln!(
				out,
				" [Upgradable from: {}]",
				config.color_ver(installed.version()),
			);
		}
	}

	// The version will not have an upgradable string, but is installed
	if version.is_installed() {
		dprint!(config, "Version is installed and not upgradable.");

		// Version isn't downloadable, consider it locally installed
		if !version.is_downloadable() {
			return writeln!(out, " [Installed, Local]");
		}

		if pkg.is_auto_removable() {
			return writeln!(out, " [Installed, Auto-Removable]");
		}

		if pkg.is_auto_installed() {
			return writeln!(out, " [Installed, Automatic]");
		}

		// None of the installed conditions were met
		return writeln!(out, " [Installed]");
	}

	// Conditions aren't met, return the package name and version
	dprint!(config, "Version meets no conditions. Will only be listed.");
	out.write_all(b"\n")
}

/// List the description or summary if requested
fn list_description(
	out: &mut impl std::io::Write,
	config: &Config,
	version: &Version,
) -> Result<()> {
	if config.get_bool("description", false) {
		writeln!(
			out,
			" {}\n",
			version
				.description()
				.unwrap_or_else(|| "No Description".to_string())
		)?;
	}
	if config.get_bool("summary", false) {
		writeln!(
			out,
			" {}\n",
			version
				.summary()
				.unwrap_or_else(|| "No Summary".to_string())
		)?;
	}
	Ok(())
}

/// List a virtual package
fn list_virtual(
	out: &mut impl std::io::Write,
	config: &Config,
	pkg: &Package,
) -> Result<(), std::io::Error> {
	write!(
		out,
		"{}{}{} ",
		config.highlight("("),
		config.color(Theme::Notice, "Virtual Package"),
		config.highlight(")")
	)?;

	if !pkg.has_provides() {
		return writeln!(out, "\n  Nothing provides this package.");
	}

	// If the virtual package provides anything show it
	writeln!(
		out,
		"\n  {} {}",
		config.highlight("Provided By:"),
		&pkg.provides()
			.map(|p| p.package().fullname(true))
			.collect::<Vec<_>>()
			.join(", "),
	)
}

#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod test {
	use crate::new_cache;
	use crate::config::Config;
	use crate::cmd::list::*;

	#[test]
	fn virtual_list() {
		let mut out = Vec::new();
		let cache = new_cache!().unwrap();
		let config = Config::default();

		// Package selection is based on current Debian Sid
		// These tests may not be consistent across distributions
		let pkg = cache.get("matemenu").unwrap();
		list_virtual(&mut out, &config, &pkg).unwrap();
		// Just a print so the output looks better in the tests
		println!("\n");

		// Convert the vector of bytes to a string
		let output = std::str::from_utf8(&out).unwrap();

		// Set up what the common string between the two tests are
		let mut virt = String::from("\u{1b}[1m(\u{1b}[0m\u{1b}[1;38;5;11m");
		virt += "Virtual Package";
		virt += "\u{1b}[0m\u{1b}[1m)\u{1b}[0m \n";

		// Create the specific string for a virtual package that provides nothing
		let mut string = virt.clone();
		string += "  Nothing provides this package.\n";

		// Test if we're correct or not
		dbg!(&output);
		dbg!(&string);
		assert_eq!(output, string);

		let mut out = Vec::new();
		let pkg = cache.get("systemd-sysusers").unwrap();

		// Do the same thing again but with a virtual package that provides
		list_virtual(&mut out, &config, &pkg).unwrap();

		// Set up what the correct output should be
		let mut string = virt.clone();
		string += "  ";
		string += &config.highlight("Provided By:");
		string += " systemd-standalone-sysusers, systemd, opensysusers\n";

		// Convert the vector of bytes to a string
		let output = std::str::from_utf8(&out).unwrap();

		dbg!(&output);
		dbg!(&string);
		assert_eq!(output, string);
	}

	#[test]
	fn description() {
		// TODO: This test is not working in the CI.
		// The full description isn't happening. Must investigate
		let mut out = Vec::new();
		let cache = new_cache!().unwrap();
		let mut config = Config::default();

		// Set the description to true so that we are able to get it
		config.set_bool("description", true);

		let pkg = cache.get("dpkg").unwrap();

		list_description(&mut out, &config, &pkg.candidate().unwrap()).unwrap();

		// Convert the vector of bytes to a string
		let output = std::str::from_utf8(&out).unwrap();

		// Match the description. This may change with different versions of dpkg
		let string = " Debian package management system\n This package provides the low-level \
		              infrastructure for handling the\n installation and removal of Debian \
		              software packages.\n .\n For Debian package development tools, install \
		              dpkg-dev.\n\n";

		dbg!(&output);
		dbg!(string);
		assert_eq!(output, string);

		// Reset and change the environment for the summary test
		let mut out = Vec::new();

		config.set_bool("description", false);
		config.set_bool("summary", true);

		list_description(&mut out, &config, &pkg.candidate().unwrap()).unwrap();

		// Convert the vector of bytes to a string
		let output = std::str::from_utf8(&out).unwrap();

		// Match the summary. This may change with different versions of dpkg
		let string = " Debian package management system\n\n";

		dbg!(&output);
		dbg!(&string);
		assert_eq!(output, string);
	}

	#[test]
	fn version() {
		let mut out = Vec::new();
		let cache = new_cache!().unwrap();
		let config = Config::default();

		let pkg = cache.get("dpkg").unwrap();
		let cand = pkg.candidate().unwrap();

		list_version(&mut out, &config, &pkg, &cand).unwrap();

		// Convert the vector of bytes to a string
		let output = std::str::from_utf8(&out).unwrap();

		// Match the description. This may change with different versions of dpkg
		let mut string = String::from("\u{1b}[1m(\u{1b}[0m\u{1b}[1;38;5;12m");
		string += cand.version();
		string += "\u{1b}[0m\u{1b}[1m)\u{1b}[0m [Debian/unstable main] [Installed, Automatic]\n";

		dbg!(&output);
		dbg!(&string);
		assert_eq!(output, string);
	}

	// TODO: Fix Test
	// #[test]
	// fn glob() {
	// 	let cache = new_cache!().unwrap();
	// 	// Results are based on Debian Sid
	// 	// These results could change and require updating

	// 	let mut packages =
	// 	glob::pkgs_with_modifiers(vec!["apt?y".to_string(),
	// "aptly*".to_string()], config, &cache)? 		.into_iter()
	// 		.filter_map(|cli| cli.pkg)
	// 		.collect::<Vec<_>>();

	// 	// Remove anything that is not amd64 arch.
	// 	// TODO: This should be dynamic based on the hosts primary arch.
	// 	packages.retain(|p| p.arch() == "amd64");

	// 	// print just for easy debugging later
	// 	for pkg in &packages {
	// 		println!("{}", pkg.fullname(false));
	// 	}
	// 	// Currently there are 3 package names that should match
	// 	assert_eq!(packages.len(), 3);
	// }

	// #[test]
	// fn regex() {
	// 	let cache = new_cache!().unwrap();
	// 	let sort = PackageSort::default().names();

	// 	// This regex should pull in only dpkg and apt
	// 	let matcher = Matcher::from_regexs(&[r"^dpk.$", r"^apt$"]).unwrap();

	// 	let (mut packages, _not_found) =
	// matcher.regex_pkgs(cache.packages(&sort), true);

	// 	packages.retain(|p| p.arch() == "amd64");

	// 	// print just for easy debugging later
	// 	for pkg_name in &packages {
	// 		println!("{}", pkg_name.name());
	// 	}
	// 	// Should only contain 2 packages, dpkg and apt
	// 	assert_eq!(packages.len(), 2);
	// }
}
