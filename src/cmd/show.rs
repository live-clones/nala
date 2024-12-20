use std::fs;

use anyhow::Result;
use rust_apt::{new_cache, BaseDep, Dependency};

use crate::config::{color, Config, Theme};
use crate::{glob, info};
use crate::util::PACSTALL;

use super::ShowVersion;

pub fn format_dependency(base_dep: &BaseDep, theme: Theme) -> String {
	if let Some(comp) = base_dep.comp_type() {
		return format!(
			// libgnutls30 (>= 3.7.5)
			"{} {}{comp} {}{}",
			// There's a compare operator in the dependency.
			// Dang better have a version smh my head.
			color::color!(theme, base_dep.target_package().name()),
			color::highlight!("("),
			color::ver!(base_dep.version().unwrap()),
			color::highlight!(")"),
		);
	}
	color::color!(theme, base_dep.target_package().name()).into()
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

pub fn show_dependency(depends: &[Dependency], theme: Theme) -> String {
	let mut depends_string = String::new();
	// Get total deps number to include Or Dependencies
	let total_deps = depends.len();

	// If there are more than 4 deps format with multiple lines
	if total_deps > 4 {
		depends_string += "\n    ";
	}

	for (i, dep) in depends.iter().enumerate() {
		// Or Deps need to be formatted slightly different.
		if dep.is_or() {
			for (j, base_dep) in dep.iter().enumerate() {
				depends_string += &format_dependency(base_dep, theme);
				if j + 1 != dep.len() {
					depends_string += " | ";
				}
			}
			depends_string += dependency_footer(total_deps, i);
			continue;
		}

		// Regular dependencies are more simple than Or
		depends_string += &format_dependency(dep.first(), theme);
		depends_string += dependency_footer(total_deps, i);
	}
	depends_string
}

pub fn format_local(pkg_name: &str) -> String {
	// Check if this could potentially be a Pacstall Package.
	let mut pac_repo = String::new();
	let postfixes = ["", "-deb", "-git", "-bin", "-app"];
	for postfix in postfixes {
		if let Ok(metadata) = fs::read_to_string(format!(
			"/var/log/pacstall/metadata/{pkg_name}{postfix}"
		)) {
			if let Some(repo) = PACSTALL.captures(&metadata) {
				pac_repo += repo.get(1).unwrap().as_str();
			} else {
				pac_repo += "https://github.com/pacstall/pacstall-programs";
			}
		}
	}
	if pac_repo.is_empty() {
		return "local install".to_string();
	}

	color::secondary!(pac_repo).into()
}

/// The show command
pub fn show(config: &Config) -> Result<()> {
	let cache = new_cache!()?;

	let mut additional_records = 0;
	// Filter virtual packages into their real package.
	let all_versions = config.get_bool("all_versions", false);
	let packages = glob::pkgs_with_modifiers(config, &cache)?.only_pkgs();
	for pkg in &packages {
		let versions = pkg.versions().map(ShowVersion::new).collect::<Vec<_>>();
		additional_records += versions.len();

		if all_versions {
			for version in &versions {
				version.show(config)?;
				additional_records -= 1;
			}
		} else if let Some(version) = versions.first() {
			version.show(config)?;
			additional_records -= 1;
		}
	}

	if additional_records != 0 {
		info!(
			"There are {} additional records. Please use the {} switch to see them.",
			color::color!(Theme::Notice, &additional_records.to_string()),
			color::color!(Theme::Notice, "'-a'"),
		);
	}

	Ok(())
}
