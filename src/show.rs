use std::fs;

use anyhow::{bail, Result};
use regex::{Regex, RegexBuilder};
use rust_apt::cache::PackageSort;
use rust_apt::new_cache;
use rust_apt::package::{BaseDep, DepType, Dependency, Package, Version};
use rust_apt::records::RecordField;
use rust_apt::util::{unit_str, NumSys};

use crate::config::Config;
use crate::util::{glob_pkgs, virtual_filter};

pub fn build_regex(pattern: &str) -> Result<Regex> {
	Ok(RegexBuilder::new(pattern).case_insensitive(true).build()?)
}

pub fn format_dependency(config: &Config, base_dep: &BaseDep, red: bool) -> String {
	let open_paren = config.color.bold("(");
	let close_paren = config.color.bold(")");

	if let Some(comp) = base_dep.comp() {
		return format!(
			// libgnutls30 (>= 3.7.5)
			"{} {open_paren}{comp} {}{close_paren}",
			config.color.dependency(base_dep.target_pkg().name(), red),
			// There's a compare operator in the dependency.
			// Dang better have a version smh my head.
			config.color.blue(base_dep.version().unwrap())
		);
	}
	config
		.color
		.dependency(base_dep.target_pkg().name(), red)
		.to_string()
}

pub fn dependency_footer(total_deps: usize, index: usize) -> &'static str {
	if total_deps > 4 {
		return "\n    ";
	}

	// Only add the comma if it isn't the last.
	if index + 1 != total_deps {
		return ", ";
	}

	" "
}

pub fn show_dependency(config: &Config, depends: &Vec<&Dependency>, red: bool) -> String {
	let mut depends_string = String::new();
	// Get total deps number to include Or Dependencies
	let total_deps = depends.len();

	// If there are more than 4 deps format with multiple lines
	if total_deps > 4 {
		depends_string += "\n    "
	}

	for (i, dep) in depends.iter().enumerate() {
		// Or Deps need to be formatted slightly different.
		if dep.is_or() {
			for (j, base_dep) in dep.base_deps.iter().enumerate() {
				depends_string += &format_dependency(config, base_dep, red);
				if j + 1 != dep.base_deps.len() {
					depends_string += " | "
				}
			}
			depends_string += dependency_footer(total_deps, i);
			continue;
		}

		// Regular dependencies are more simple than Or
		depends_string += &format_dependency(config, dep.first(), red);
		depends_string += dependency_footer(total_deps, i);
	}
	depends_string
}

pub fn format_local(pkg: &Package, config: &Config, pacstall_regex: &Regex) -> String {
	// Check if this could potentially be a Pacstall Package.
	let mut pac_repo = String::new();
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
		return "local install".to_string();
	}

	config.color.blue(&pac_repo).to_string()
}

/// The show command
pub fn show_version<'a>(
	config: &Config,
	pkg: &'a Package,
	ver: &'a Version<'a>,
	pacstall_regex: &Regex,
	url_regex: &Regex,
) -> Result<()> {
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

	println!(
		"{} {}",
		config.color.bold("Download-Size:"),
		unit_str(ver.size(), NumSys::Binary)
	);

	// Package File Section
	if let Some(pkg_file) = ver.package_files().next() {
		println!(
			"{} {}",
			config.color.bold("Origin:"),
			pkg_file.origin().unwrap_or("Unknown")
		);

		// Check if source is local, pacstall or from a repo
		let mut source = String::new();
		if let Ok(archive) = pkg_file.archive() {
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
			println!("{} {source}", config.color.bold("APT-Sources:"));
		}
	}

	if let Some(record) = ver.get_record(RecordField::Homepage) {
		println!("{} {}", config.color.bold("Homepage:"), record);
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
			let red = matches!(deptype, DepType::Conflicts | DepType::Breaks);

			println!(
				"{} {}",
				config.color.bold(header),
				// Trim end because I have no idea how to code properly
				show_dependency(config, &deduped_depends, red).trim_end(),
			);
		}
	}

	println!(
		"{} {}",
		config.color.bold("Description:"),
		ver.description().unwrap_or_else(|| "Unknown".to_string())
	);

	println!();
	Ok(())
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

	let mut additional_records = 0;
	// Filter virtual packages into their real package.
	for pkg in virtual_filter(packages, &cache, config)? {
		let versions = pkg.versions().collect::<Vec<_>>();
		additional_records += versions.len();

		match config.get_bool("all-versions", false) {
			false => {
				if let Some(version) = versions.first() {
					show_version(config, &pkg, version, &pacstall_regex, &url_regex)?;
					additional_records -= 1;
				}
			},
			true => {
				for version in &versions {
					show_version(config, &pkg, version, &pacstall_regex, &url_regex)?;
					additional_records -= 1;
				}
			},
		}
	}

	if additional_records != 0 {
		config.color.notice(
			&format_args!(
				"There are {} additional records. Please use the {} switch to see them.",
				config.color.yellow(&additional_records.to_string()),
				config.color.yellow("'-a'"),
			)
			.to_string(),
		);
	}

	Ok(())
}
