use std::fs;

use anyhow::Result;
use regex::Regex;
use rust_apt::{new_cache, BaseDep, Dependency, Package, Version};

use crate::config::{Config, Theme};
use crate::glob;
use crate::util::virtual_filter;
use crate::cmd::show_version;

use super::build_regex;

pub fn format_dependency(config: &Config, base_dep: &BaseDep, theme: Theme) -> String {
	let open_paren = config.highlight("(");
	let close_paren = config.highlight(")");

	let target_name = config.color(theme, base_dep.target_package().name());

	if let Some(comp) = base_dep.comp_type() {
		return format!(
			// libgnutls30 (>= 3.7.5)
			"{target_name} {open_paren}{comp} {}{close_paren}",
			// There's a compare operator in the dependency.
			// Dang better have a version smh my head.
			config.color(Theme::Secondary, base_dep.version().unwrap())
		);
	}

	target_name
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

pub fn show_dependency(config: &Config, depends: &[&Dependency], theme: Theme) -> String {
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
				depends_string += &format_dependency(config, base_dep, theme);
				if j + 1 != dep.len() {
					depends_string += " | ";
				}
			}
			depends_string += dependency_footer(total_deps, i);
			continue;
		}

		// Regular dependencies are more simple than Or
		depends_string += &format_dependency(config, dep.first(), theme);
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
				pac_repo += repo.get(1).unwrap().as_str();
			} else {
				pac_repo += "https://github.com/pacstall/pacstall-programs";
			}
		}
	}

	if pac_repo.is_empty() {
		return "local install".to_string();
	}

	config.color(Theme::Secondary, &pac_repo).to_string()
}

pub fn print_show_version<'a>(
	config: &Config,
	pkg: &'a Package,
	ver: &'a Version,
	pacstall_regex: &Regex,
	url_regex: &Regex,
) {
	let delimiter = config.highlight(":");
	for (header, info) in show_version(config, pkg, ver, pacstall_regex, url_regex) {
		println!("{}{delimiter} {info}", config.highlight(header))
	}
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
	let packages = glob::pkgs_with_modifiers(config, &cache)?.only_pkgs();

	let mut additional_records = 0;
	// Filter virtual packages into their real package.
	for pkg in virtual_filter(packages, &cache, config)? {
		let versions = pkg.versions().collect::<Vec<_>>();
		additional_records += versions.len();

		if config.get_bool("all_versions", false) {
			for version in &versions {
				print_show_version(config, &pkg, version, &pacstall_regex, &url_regex);
				additional_records -= 1;
			}
		} else if let Some(version) = versions.first() {
			print_show_version(config, &pkg, version, &pacstall_regex, &url_regex);
			additional_records -= 1;
		}
	}

	if additional_records != 0 {
		let notice = format!(
			"There are {} additional records. Please use the {} switch to see them.",
			config.color(Theme::Notice, &additional_records.to_string()),
			config.color(Theme::Notice, "'-a'"),
		);
		config.stderr(Theme::Notice, &notice);
	}

	Ok(())
}
