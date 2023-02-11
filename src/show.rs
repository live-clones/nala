use std::collections::HashSet;
use std::fmt::format;

use anyhow::{bail, Result};
use rust_apt::cache::{Cache, PackageSort};
use rust_apt::new_cache;
use rust_apt::package::{Package, Provider, Version};

use crate::colors::Color;
use crate::config::Config;
use crate::dprint;
use crate::list::glob_pkgs;

/// The show command
pub fn show(config: &Config) -> Result<()> {
	// let mut out = std::io::stdout().lock();
	let cache = new_cache!()?;

	// Filter the packages by names if they were provided
	let sort = PackageSort::default().include_virtual();

	let (packages, _not_found) = match config.pkg_names() {
		Some(pkg_names) => glob_pkgs(pkg_names, cache.packages(&sort))?,
		None => bail!("At least one package name must be specified"),
	};

	// Extract this into a function. probably should go into util.rs
	// Can use glob_pkgs as a template on however that takes iterators.
	let mut virtual_filtered = vec![];
	for pkg in packages {
		// If the package has versions then it isn't virtual
		// just push it and continue
		if pkg.has_versions() {
			virtual_filtered.push(pkg);
			continue;
		}

		// If the package doesn't have provides it's purely virtual
		// There is nothing that can satisfy it. Referenced only by name
		// At time of commit `python3-libmapper` is purely virtual
		if !pkg.has_provides() {
			config.color.warn(&format!(
				"{} has no providers and is purely virutal",
				config.color.package(pkg.name())
			));
			continue;
		}

		// Package is virtual so get its providers.
		// HashSet for duplicated packages when there is more than one version
		let providers: HashSet<Package> = pkg.provides().map(|p| p.package()).collect();

		// If there is only one provider just select that as the target
		if providers.len() == 1 {
			// Unwrap should be fine here, we know that there is 1 in the Vector.
			let target = providers.into_iter().next().unwrap();
			config.color.notice(&format!(
				"Selecting {} instead of virtual package {}",
				config.color.package(target.name()),
				config.color.package(pkg.name())
			));

			// Unwrap should be fine here because we know the name.
			// We have to grab the package from the cache again because
			// Provider lifetimes are a bit goofy.
			virtual_filtered.push(cache.get(&target.fullname(false)).unwrap());
			continue;
		}

		// If there are multiple providers then we will error out
		// and show the packages the user could select instead.
		if providers.len() > 1 {
			println!(
				"{} is a virtual package provided by:",
				config.color.package(pkg.name())
			);
			for target in &providers {
				// If the version doesn't have a candidate no sense in showing it
				if let Some(cand) = target.candidate() {
					println!(
						"    {} {}",
						config.color.package(&target.fullname(true)),
						config.color.version(cand.version()),
					)
				}
			}
			bail!("You should select just one.")
		}
	}

	return Ok(());
	for pkg in packages {
		// Temp for development lol
		// if pkg.name() != "steam" { continue; }

		println!(
			"{} {}",
			config.color.bold("Package:"),
			config.color.package(pkg.name()),
		);
		// This package is completely virtual. Exists only in reference
		println!("{} {}", config.color.bold("Virtual:"), !pkg.has_versions());

		// If there are provides then show them!
		if pkg.has_provides() {
			println!(
				"{}",
				config
					.color
					.bold("Packages that provide this virtual package:")
			);

			// put the package names in a HashSet so there aren't duplicates
			// this happens if there are multiple versions of the same package
			let providers: HashSet<String> = pkg
				.provides()
				.map(|p| p.package().fullname(false))
				.collect();

			for pkg_name in providers {
				// Print the package name that is provided
				println!("    {}", config.color.package(&pkg_name));
			}
		}

		println!("Id: {}", pkg.id());
		println!("Architecture: {}", pkg.arch());
		for version in pkg.versions() {
			println!("Architecture: {}", version.arch());
			println!("Version: {}", version.version());
		}

		println!("\n");
	}

	Ok(())
}
