use std::collections::HashMap;
use std::fmt::format;
use std::fs;
use std::hash::Hash;
use std::ops::Index;
use std::path::PathBuf;

use anyhow::{bail, Result};
use regex::{Regex, RegexBuilder};
use rust_apt::cache::PackageSort;
use rust_apt::new_cache;
use rust_apt::package::{DepType, Dependency};
use rust_apt::records::RecordField;
use rust_apt::util::{unit_str, NumSys};

use crate::config::Config;
use crate::util::{glob_pkgs, virtual_filter};

pub fn build_regex(pattern: &str) -> Result<Regex> {
	Ok(RegexBuilder::new(pattern).case_insensitive(true).build()?)
}

pub fn show_dependency(config: &Config, depends: &Vec<Dependency>, red: bool) -> String {
	let mut depends_string = String::new();

	if depends.len() > 4 {
		depends_string += "\n    "
	}

	for dep in depends {
		let mut dep_string = String::new();
		// Or Deps need to be formatted slightly different.
		if dep.is_or() {
			continue;
		}

		let base_dep = dep.first();

		let open_paren = config.color.bold("(");
		let close_paren = config.color.bold(")");

		if let Some(comp) = base_dep.comp() {
			dep_string += &format!(
				// libgnutls30 (>= 3.7.5)
				"{} {open_paren}{comp} {}{close_paren}",
				config.color.dependency(&base_dep.name(), red),
				// There's a compare operator in the dependency.
				// Dang better have a version smh my head.
				config.color.blue(base_dep.version().unwrap())
			);
		} else {
			dep_string += &config.color.dependency(&base_dep.name(), red);
		}

		if depends.len() > 4 {
			dep_string += "\n    "
		} else {
			dep_string += " "
		}
		depends_string += &dep_string;
	}
	depends_string.trim_end().to_string()
}

/// The show command
pub fn show(config: &Config) -> Result<()> {
	// let mut out = std::io::stdout().lock();
	let cache = new_cache!()?;

	// Regex for formating the Apt sources from URI.
	let url_regex = build_regex("(https?://.*?/.*?/)")?;
	// Regex for finding Pacstall remote repo
	let pacstall_regex = build_regex(r#"_remoterepo="(.*?)""#)?;

	// Filter the packages by names if they were provided
	let sort = PackageSort::default().include_virtual();

	let (packages, _not_found) = match config.pkg_names() {
		Some(pkg_names) => glob_pkgs(pkg_names, cache.packages(&sort))?,
		None => bail!("At least one package name must be specified"),
	};

	// Filter virtual packages into their real package.
	for pkg in virtual_filter(packages, &cache, config)? {
		// Because of the virtual filter, no virtual packages should make it here.
		let ver = pkg.versions().next().unwrap();
		// Temp change to installed for Pacstall testing.
		// let ver = pkg.versions().last().unwrap();

		// let mut show_map = HashMap::new();
		// let mut show_map: HashMap<&str, String> = HashMap::from([
		// 	("Package", pkg.fullname(true)),
		// 	("Version", config.color.blue(ver.version())),
		// 	("Architecture", pkg.arch().to_string()),
		// 	("Installed", pkg.installed().is_some().to_string()),
		// 	("Priority", priority.to_string()),
		// 	("Essential", pkg.is_essential().to_string()),
		// 	("Section", "contrib/oldlibs".to_string()),
		// 	("Source", ver.source_name().to_string()),
		// 	(
		// 		"Origin",
		// 		ver.package_files()
		// 			.next()
		// 			.unwrap()
		// 			.origin()
		// 			.unwrap()
		// 			.to_string(),
		// 	),
		// ("Maintainer",),
		// ("Installed-Size",),
		// Maybe need to format the Provides the same as depends?
		// ("Provides",),
		// Need to figure out how I'm going to format the depends.
		// ("Depends",),
		// ("Homepage",),
		// ("Download-Size",),
		// ("APT-Sources",),
		// ("Description",),
		// ]);

		// for (header, value) in show_map {
		// 	println!("{} {value}", config.color.bold(header))
		// }
		println!(
			"{} {}",
			config.color.bold("Package:"),
			config.color.package(&pkg.fullname(true))
		);

		println!(
			"{} {}",
			config.color.bold("Version:"),
			config.color.blue(ver.version())
		);

		println!("{} {}", config.color.bold("Architecture:"), pkg.arch());

		println!(
			"{} {}",
			config.color.bold("Installed:"),
			if ver.is_installed() { "Yes" } else { "No" }
		);

		println!(
			"{} {}",
			config.color.bold("Priority:"),
			ver.priority_str().unwrap_or("Unknown")
		);

		println!(
			"{} {}",
			config.color.bold("Essential:"),
			if pkg.is_essential() { "Yes" } else { "No" }
		);

		println!(
			"{} {}",
			config.color.bold("Section:"),
			ver.section().unwrap_or("Unknown")
		);

		println!("{} {}", config.color.bold("Source:"), ver.source_name());

		if let Some(record) = ver.get_record(RecordField::Maintainer) {
			println!("{} {}", config.color.bold("Maintainer:"), record);
		}

		if let Some(record) = ver.get_record(RecordField::OriginalMaintainer) {
			println!("{} {}", config.color.bold("Original-Maintainer:"), record);
		}

		println!(
			"{} {}",
			config.color.bold("Installed-Size:"),
			unit_str(ver.installed_size(), NumSys::Binary)
		);

		// Package File Section
		if let Some(pkg_file) = ver.package_files().next() {
			println!(
				"{} {}",
				config.color.bold("Origin:"),
				pkg_file.origin().unwrap_or("Unknown")
			);
		}

		// Add Depends here, Not sure how I wanna do the dang thing.
		// Second line comment so I extra don't forget.
		// Will probably still forget.

		if let Some(record) = ver.get_record(RecordField::Homepage) {
			println!("{} {}", config.color.bold("Homepage:"), record);
		}

		println!(
			"{} {}",
			config.color.bold("Download-Size:"),
			unit_str(ver.size(), NumSys::Binary)
		);

		// Formating APT-Source. This is probably going to need extraction.
		// We too will be adding support for Pacstall packages as python Nala has
		if let Some(pkg_file) = ver.package_files().next() {
			let mut source = String::new();
			let mut pac_repo = String::new();
			if let Ok(archive) = pkg_file.archive() {
				if archive == "now" {
					// Check if this could potentially be a Pacstall Package.
					let postfixes = ["", "-deb", "-git", "-bin", "-app"];
					for postfix in postfixes {
						if let Ok(metadata) = fs::read_to_string(format!(
							"/var/log/pacstall/metadata/{}{}",
							pkg.name(),
							postfix
						)) {
							if let Some(repo) = pacstall_regex.captures(&metadata) {
								pac_repo += repo.get(1).unwrap().as_str()
							} else {
								pac_repo += "https://github.com/pacstall/pacstall-programs"
							}
						}
					}

					if pac_repo.is_empty() {
						source += "local install"
					} else {
						source += &config.color.blue(&pac_repo)
					}
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
				println!("{} {source}", config.color.bold("APT-Sources:"));
			}
		}

		// If there are provides then show them!
		let providers: Vec<String> = ver
			.provides()
			.map(|p| config.color.package(p.name()).to_string())
			.collect();

		if !providers.is_empty() {
			println!("{} {}", config.color.bold("Provides:"), providers.join(" "));
		}

		let dependencies = [
			("Depends:", DepType::Depends),
			("Recommends:", DepType::Recommends),
			("Suggests:", DepType::Suggests),
			("Replaces:", DepType::Replaces),
			("Conflicts:", DepType::Conflicts),
			("Breaks:", DepType::Breaks),
		];

		for (header, deptype) in dependencies {
			// These Dependency types will be colored red
			let red = match deptype {
				DepType::Conflicts => true,
				DepType::Breaks => true,
				_ => false,
			};

			if let Some(depends) = ver.get_depends(&deptype) {
				println!(
					"{} {}",
					config.color.bold(header),
					show_dependency(config, depends, red),
				);
			}
		}

		println!(
			"{} {}",
			config.color.bold("Description:"),
			ver.description().unwrap_or_else(|| "Unknown".to_string())
		);

		println!("\n");
	}

	Ok(())
}
