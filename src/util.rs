#[macro_export]
/// Print Debug information if the option is set
macro_rules! dprint {
	($config:expr $(,)?, $($arg: tt)*) => {
		if $config.debug() {
			let string = std::fmt::format(std::format_args!($($arg)*));
			eprintln!("DEBUG: {string}");
		}
	};
}

#[macro_export]
/// Print Debug information using NalaProgress.
macro_rules! dprog {
	($config:expr, $progress:expr, $context:expr, $(,)? $($arg:tt)*) => {
		if $config.debug() {
			let output = std::fmt::format(std::format_args!($($arg)*));
			if $progress.hidden() {
				eprintln!("DEBUG({}): {output}", $context);
			} else {
				$progress.print(&format!("DEBUG({}): {output}", $context))?;
			}
		}
	};
}

use std::cell::OnceCell;
use std::collections::HashSet;

use anyhow::{bail, Result};
use regex::{Regex, RegexBuilder};
use rust_apt::records::RecordField;
use rust_apt::{Cache, Package, Version};

use crate::colors::Theme;
use crate::config::Config;

pub struct NalaRegex {
	mirror: OnceCell<Regex>,
	domain: OnceCell<Regex>,
	ubuntu_url: OnceCell<Regex>,
	ubuntu_country: OnceCell<Regex>,
}

impl NalaRegex {
	pub fn new() -> Self {
		NalaRegex {
			mirror: OnceCell::new(),
			domain: OnceCell::new(),
			ubuntu_url: OnceCell::new(),
			ubuntu_country: OnceCell::new(),
		}
	}

	fn build_regex(regex: &str) -> Regex {
		RegexBuilder::new(regex)
			.case_insensitive(true)
			.build()
			.unwrap()
	}

	pub fn mirror(&self) -> &Regex {
		self.mirror
			.get_or_init(|| Self::build_regex(r"(mirror://(.*?)/pool|mirror\+file:(/.*?)/pool)"))
	}

	pub fn domain(&self) -> &Regex {
		self.domain
			.get_or_init(|| Self::build_regex(r"https?://([A-Za-z_0-9.-]+).*"))
	}

	pub fn ubuntu_url(&self) -> &Regex {
		self.ubuntu_url
			.get_or_init(|| Self::build_regex(r"<link>(.*)</link>"))
	}

	pub fn ubuntu_country(&self) -> &Regex {
		self.ubuntu_country
			.get_or_init(|| Self::build_regex(r"<mirror:countrycode>(.*)</mirror:countrycode>"))
	}
}

pub fn virtual_filter<'a, Container: IntoIterator<Item = Package<'a>>>(
	packages: Container,
	cache: &'a Cache,
	config: &Config,
) -> Result<HashSet<Package<'a>>> {
	// clippy thinks that the package is mutable
	// But it only hashes the ID and you can't really mutate a package
	#[allow(clippy::mutable_key_type)]
	let mut virtual_filtered = HashSet::new();
	for pkg in packages {
		// If the package has versions then it isn't virtual
		// just push it and continue
		if pkg.has_versions() {
			virtual_filtered.insert(pkg);
			continue;
		}

		// If the package doesn't have provides it's purely virtual
		// There is nothing that can satisfy it. Referenced only by name
		// At time of commit `python3-libmapper` is purely virtual
		if !pkg.has_provides() {
			config.color(
				Theme::Warning,
				&format!(
					"{} has no providers and is purely virutal",
					config.color(Theme::Primary, pkg.name())
				),
			);
			continue;
		}

		// Package is virtual so get its providers.
		// HashSet for duplicated packages when there is more than one version
		// clippy thinks that the package is mutable
		// But it only hashes the ID and you can't really mutate a package
		#[allow(clippy::mutable_key_type)]
		let providers: HashSet<Package> = pkg.provides().map(|p| p.package()).collect();

		// If there is only one provider just select that as the target
		if providers.len() == 1 {
			// Unwrap should be fine here, we know that there is 1 in the Vector.
			let target = providers.into_iter().next().unwrap();
			config.color(
				Theme::Notice,
				&format!(
					"Selecting {} instead of virtual package {}",
					config.color(Theme::Primary, &target.fullname(false)),
					config.color(Theme::Primary, pkg.name())
				),
			);

			// Unwrap should be fine here because we know the name.
			// We have to grab the package from the cache again because
			// Provider lifetimes are a bit goofy.
			virtual_filtered.insert(cache.get(&target.fullname(false)).unwrap());
			continue;
		}

		// If there are multiple providers then we will error out
		// and show the packages the user could select instead.
		if providers.len() > 1 {
			println!(
				"{} is a virtual package provided by:",
				config.color(Theme::Primary, pkg.name())
			);
			for target in &providers {
				// If the version doesn't have a candidate no sense in showing it
				if let Some(cand) = target.candidate() {
					println!(
						"    {} {}",
						config.color(Theme::Primary, &target.fullname(true)),
						config.color_ver(cand.version()),
					);
				}
			}
			bail!("You should select just one.")
		}
	}
	Ok(virtual_filtered)
}

pub fn version_diff(config: &Config, old: &str, new: String) -> String {
	// Check for just revision change first.
	if let (Some(old_ver), Some(new_ver)) = (old.rsplit_once('-'), new.rsplit_once('-')) {
		// If there isn't a revision these shouldn't ever match
		// If they do match then only the revision has changed
		if old_ver.0 == new_ver.0 {
			return format!("{}-{}", new_ver.0, config.color(Theme::Notice, new_ver.0));
		}
	}

	let (old_ver, new_ver) = (
		old.split('.').collect::<Vec<_>>(),
		new.split('.').collect::<Vec<_>>(),
	);

	let mut start_color = 0;
	for (i, section) in old_ver.iter().enumerate() {
		if i > new_ver.len() - 1 {
			break;
		}

		if section != &new_ver[i] {
			start_color = i;
			break;
		}
	}

	new_ver
		.iter()
		.enumerate()
		.map(|(i, str)| {
			if i >= start_color {
				config.color(Theme::Notice, str)
			} else {
				str.to_string()
			}
		})
		.collect::<Vec<_>>()
		.join(".")
}

/// Return the package name. Checks if epoch is needed.
pub fn get_pkg_name(version: &Version) -> String {
	let filename = version
		.get_record(RecordField::Filename)
		.expect("Record does not contain a filename!")
		.split_terminator('/')
		.last()
		.expect("Filename is malformed!")
		.to_string();

	if let Some(index) = version.version().find(':') {
		let epoch = format!("_{}%3a", &version.version()[..index]);
		return filename.replacen('_', &epoch, 1);
	}
	filename
}

#[link(name = "c")]
extern "C" {
	pub fn geteuid() -> u32;
}

/// Check for root. Errors if not root.
/// Set up lock file if root.
pub fn sudo_check(config: &Config) -> Result<()> {
	if unsafe { geteuid() != 0 } {
		bail!("Nala needs root to {}", config.command)
	}
	// TODO: Need to add lock file logic here maybe.
	Ok(())
}

/// Get the username or return Unknown.
pub fn get_user() -> (std::string::String, std::string::String) {
	let uid = std::env::var("SUDO_UID").unwrap_or_else(|_| format!("{}", unsafe { geteuid() }));

	let username = std::env::var("SUDO_USER").unwrap_or_else(|_| {
		for key in ["LOGNAME", "USER", "LNAME", "USERNAME"] {
			if let Ok(name) = std::env::var(key) {
				return name;
			}
		}
		"Unknown".to_string()
	});

	(uid, username)
}
