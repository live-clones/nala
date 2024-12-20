use anyhow::{bail, Result};
use rust_apt::new_cache;
use rust_apt::util::show_broken_pkg;

use crate::config::Config;
use crate::deb::DebFile;
use crate::download::Downloader;
use crate::glob::CliPackage;
use crate::cmd::Operation;
use crate::util::sudo_check;
use crate::{debug, glob, info};
use crate::config::color;

#[tokio::main]
pub async fn install(config: &Config) -> Result<()> {
	debug!("Install");
	sudo_check(config)?;

	let mut http_pkgs = vec![];
	let mut deb_files = vec![];
	let mut cache_pkgs = vec![];

	// TODO: Maybe this could be better
	// For example maybe not having to hold ownership
	// deduped list stored in the config object?
	let cmd_line_pkgs = config.pkg_names()?;
	for pkg in cmd_line_pkgs {
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
		deb_files.push(DebFile::new(pkg).await?);
	}

	// TODO: This is a little clunky because of DebFile
	// holding a reference to the cli_uris
	//
	// Download packages from http before anything else
	// so we know the dependencies for resolving other packages
	let uris = if !http_pkgs.is_empty() {
		let mut downloader = Downloader::new(config)?;
		for pkg in http_pkgs {
			downloader.add_from_cmdline(&pkg).await?;
		}
		downloader.run(config, true).await?
	} else {
		vec![]
	};

	for uri in &uris {
		if config.verbose() {
			println!("Downloaded: {:?}", uri.archive)
		}
		deb_files.push(DebFile::new(uri.archive.to_string_lossy().into()).await?);
	}

	let deb_paths: Vec<&str> = deb_files.iter().map(|deb| deb.path.as_str()).collect();
	let cache = new_cache!(&deb_paths)?;

	let mut packages = glob::pkgs_with_modifiers(config, &cache)?;

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
		packages.push(CliPackage::new_glob(pkg.name().to_string())?.with_pkg(pkg, ver))
	}

	for found in packages.found() {
		let pkg = &found.pkg;
		match found.modifier.unwrap_or(Operation::Install) {
			Operation::Install => {
				let Some(cand) = pkg.candidate() else {
					bail!("{} has no install candidate", pkg.name())
				};

				if let Some(inst) = pkg.installed() {
					if inst == cand {
						info!(
							"{}{} is already installed and at the latest version",
							color::primary!(pkg.name()),
							color::ver!(cand.version())
						);
						continue;
					}
				}
				cache.resolver().clear(pkg);
				cache.resolver().protect(pkg);
				pkg.mark_install(true, true);
			},
			Operation::Remove => {
				let Some(_inst) = pkg.installed() else {
					info!("{} is not installed", pkg.name());
					continue;
				};

				// TODO: Apt has this, I think we need to bind this in rust-apt though
				// Potentially can call it pkg.mark_hold()?
				//
				// MarkInstall refuses to install packages on hold
				// Pkg->SelectedState = pkgCache::State::Hold;

				// TODO: Configure so we can purge >:)
				debug!("Mark Delete: {pkg}");
				cache.resolver().clear(pkg);
				cache.resolver().protect(pkg);
				pkg.mark_delete(false);
			},
			_ => todo!(),
		}
	}

	if let Err(err) = cache.resolve(false) {
		println!("Broken Count: {}", cache.depcache().broken_count());
		for pkg in cache.iter() {
			if let Some(broken) = show_broken_pkg(&cache, &pkg, false) {
				eprintln!("{broken}");
			};
		}
		bail!(err);
	}

	crate::summary::commit(cache, config).await
}
