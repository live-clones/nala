use anyhow::{bail, Result};
use rust_apt::{new_cache, PackageSort};

use crate::colors::Theme;
use crate::config::Config;
use crate::debfile::DebFile;
use crate::download::Downloader;
use crate::dprint;
use crate::util::{glob_pkgs, sudo_check};

#[tokio::main]
pub async fn install(config: &Config) -> Result<()> {
	dprint!(config, "Install");
	sudo_check(config)?;

	let mut http_pkgs = vec![];
	let mut deb_files = vec![];
	let mut cache_pkgs = vec![];

	// TODO: Maybe this could be better
	// For example maybe not having to hold ownership
	// deduped list stored in the config object?
	let cmd_line_pkgs = config.pkg_names()?;
	for pkg in &cmd_line_pkgs {
		// TODO: Make Http actually do something
		// Look at python Nala for hints on features
		if pkg.starts_with("http") {
			http_pkgs.push(pkg);
			continue;
		}

		// Treat it as pkg name if it isn't .deb
		if !pkg.ends_with(".deb") {
			cache_pkgs.push(pkg);
			continue;
		}

		// All else are local
		deb_files.push(DebFile::new(pkg)?);
	}

	// TODO: This is a little clunky because of DebFile
	// holding a reference to the cli_uris
	//
	// Download packages from http before anything else
	// so we know the dependencies for resolving other packages
	let uris = if !http_pkgs.is_empty() {
		let mut downloader = Downloader::new(config)?;
		for pkg in http_pkgs {
			downloader.add_from_cmdline(config, pkg).await?;
		}
		downloader.run(config, true).await?
	} else {
		vec![]
	};

	for uri in &uris {
		if config.verbose() {
			println!("Downloaded: {:?}", uri.archive)
		}
		deb_files.push(DebFile::new(&uri.archive)?)
	}

	let deb_paths: Vec<&str> = deb_files.iter().map(|deb| deb.path).collect();
	let cache = new_cache!(&deb_paths)?;

	let sort = PackageSort::default().include_virtual();
	let (mut packages, not_found) = glob_pkgs(&cache_pkgs, cache.packages(&sort))?;

	packages.sort_by_cached_key(|pkg| pkg.name().to_string());

	if !not_found.is_empty() {
		for name in &not_found {
			config.stderr(
				Theme::Error,
				&format!("'{}' was not found", config.color(Theme::Notice, name)),
			);
		}
		bail!("Some packages were not found in the cache")
	}

	// Fetch the correct local .deb and version from the cache
	for deb in deb_files {
		let Some(pkg) = cache.get(deb.name()) else {
			continue;
		};

		println!(
			"Selecting Package '{}' instead of '{}'",
			pkg.name(),
			deb.path
		);
		let Some(ver) = pkg.get_version(deb.version()) else {
			bail!(
				"Could not find Version '{}' for Package '{}'",
				deb.version(),
				pkg.name()
			)
		};
		ver.set_candidate();
		packages.push(pkg)
	}

	for pkg in &packages {
		let Some(cand) = pkg.candidate() else {
			bail!("{} has no install candidate", pkg.name())
		};

		if let Some(inst) = pkg.installed() {
			if inst == cand {
				let pkg_name = config.color(Theme::Primary, pkg.name());
				let ver = config.color_ver(cand.version());

				config.stderr(
					Theme::Notice,
					&format!("{pkg_name}{ver} is already installed and at the latest version"),
				);
				continue;
			}
		}
		pkg.mark_install(true, true);
	}

	cache.resolve(false)?;
	crate::summary::commit(cache, config).await
}
