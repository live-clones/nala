use std::collections::HashMap;

use anyhow::{bail, Result};
use chrono::Utc;
use rust_apt::util::DiskSpace;
use rust_apt::{Cache, Marked};

use crate::cmd::{
	self, apt_hook_with_pkgs, ask, auto_remover, run_scripts, HistoryEntry, HistoryPackage,
	Operation,
};
use crate::config::Config;
use crate::download::Downloader;
use crate::{dpkg, dprint, table, tui};

/// Run the autoremover and then get the changes from the cache.
pub fn get_changes(
	cache: &Cache,
	config: &Config,
) -> Result<HashMap<Operation, Vec<HistoryPackage>>> {
	dprint!(config, "Running auto_remover");
	let auto_remove = auto_remover(cache);

	let mut pkg_set: HashMap<Operation, Vec<HistoryPackage>> = HashMap::new();

	dprint!(config, "Calculating changes");
	let changed = cache.get_changes(true).collect::<Vec<_>>();
	if changed.is_empty() {
		return Ok(pkg_set);
	}

	for pkg in &changed {
		let (op, ver) = match pkg.marked() {
			mark @ (Marked::NewInstall | Marked::Install | Marked::ReInstall) => {
				let Some(cand) = pkg.install_version() else {
					continue;
				};
				let op = match mark {
					Marked::ReInstall => Operation::Reinstall,
					_ => Operation::Install,
				};
				(op, cand)
			},
			Marked::Remove | Marked::Purge => {
				let Some(inst) = pkg.installed() else {
					continue;
				};

				if auto_remove.contains(&inst) {
					continue;
				}

				let op = if pkg.marked_purge() { Operation::Purge } else { Operation::Remove };
				(op, inst)
			},
			mark @ (Marked::Upgrade | Marked::Downgrade) => {
				if let (Some(inst), Some(cand)) = (pkg.installed(), pkg.candidate()) {
					let op = match mark {
						Marked::Upgrade => Operation::Upgrade,
						_ => Operation::Downgrade,
					};

					pkg_set
						.entry(op)
						.or_default()
						.push(HistoryPackage::from_version(op, &cand, &Some(inst)));
				}
				continue;
			},
			// TODO: See if pkg is held for phasing and show percent
			// pkgDepCache::PhasingApplied
			// VerIterator::PhasedUpdatePercentage
			Marked::Held => {
				let Some(cand) = pkg.candidate() else {
					continue;
				};
				(Operation::Held, cand)
			},
			Marked::Keep => continue,
			Marked::None => bail!("{pkg} not marked, this should be impossible"),
		};

		pkg_set
			.entry(op)
			.or_default()
			.push(HistoryPackage::from_version(op, &ver, &None));
	}

	if !auto_remove.is_empty() {
		pkg_set.insert(
			Operation::AutoRemove,
			auto_remove
				.into_iter()
				.map(|v| HistoryPackage::from_version(Operation::AutoRemove, &v, &None))
				.collect(),
		);
	}

	Ok(pkg_set)
}

/// TODO: Implement a simple summary that is very short for serial/console users
pub async fn display_summary(
	cache: &Cache,
	config: &Config,
	pkg_set: &HashMap<Operation, Vec<HistoryPackage>>,
) -> Result<bool> {
	if config.get_no_bool("tui", true) {
		// App returns true if we should continue.
		tui::summary::SummaryTab::new(cache, config, pkg_set)
			.run()
			.await
	} else {
		let mut tables = vec![];

		for (op, pkgs) in pkg_set {
			let mut table = table::get_table(
				config,
				if pkgs[0].items(config).len() > 3 {
					&["Package:", "Old Version:", "New Version:", "Size:"]
				} else {
					&["Package:", "Version:", "Size:"]
				},
			);

			table.add_rows(pkgs.iter().map(|p| p.items(config)));
			tables.push((op, table));
		}

		let width = rust_apt::util::terminal_width();
		let sep = "=".repeat(width);

		for (op, pkgs) in tables {
			println!("{sep}");
			println!(" {}", config.highlight(op.as_str()));
			println!("{sep}");

			println!("{pkgs}");
		}
		println!("{sep}");
		println!(" Summary");
		println!("{sep}");

		for (op, pkgs) in pkg_set {
			println!(" {op} {}", pkgs.len())
		}

		println!();
		if cache.depcache().download_size() > 0 {
			println!(
				" Total download size: {}",
				config.unit_str(cache.depcache().download_size())
			)
		}

		match cache.depcache().disk_size() {
			DiskSpace::Require(disk_space) => {
				println!(" Disk space required: {}", config.unit_str(disk_space))
			},
			DiskSpace::Free(disk_space) => {
				println!(" Disk space to free: {}", config.unit_str(disk_space))
			},
		}
		println!();

		// Returns an error if yes is no selected
		ask("Do you want to continue?")?;
		Ok(true)
	}
}

pub async fn commit(cache: Cache, config: &Config) -> Result<()> {
	let pkg_set = crate::summary::get_changes(&cache, config)?;
	if pkg_set.is_empty() {
		println!("Nothing to do.");
		return Ok(());
	}

	let changed = cache.get_changes(true).collect::<Vec<_>>();
	let versions = changed
		.iter()
		.filter_map(|pkg| pkg.install_version())
		.collect::<Vec<_>>();

	let mut downloader = Downloader::new(config)?;
	for ver in &versions {
		downloader.add_version(ver, config).await?;
	}

	if config.get_bool("print_uris", false) {
		for uri in downloader.uris() {
			println!("{}", uri.to_json()?);
		}
		// Print uris does not go past here
		return Ok(());
	};

	if !crate::summary::display_summary(&cache, config, &pkg_set).await? {
		return Ok(());
	};

	// Only download if needed
	// Downloader will error if empty download
	// TODO: Should probably just make run check and return Ok(vec![])?
	if !downloader.uris().is_empty() {
		let _finished = downloader.run(config, false).await?;
	}

	if config.get_bool("download_only", false) {
		return Ok(());
	}

	let history_entry = HistoryEntry::new(
		cmd::get_history(config)
			.await?
			.iter()
			.map(|entry| entry.id)
			.max()
			.unwrap_or_default()
			+ 1,
		Utc::now().to_rfc3339(),
		pkg_set.into_values().flatten().collect(),
	);

	history_entry.write_to_file(config)?;

	// TODO: There should likely be a field in the history
	// to mark that it was a transaction that failed.
	// The idea is to run the rest of this program,
	// catch any errors, and then write the history file
	// Either way but we'll know that it failed.

	run_scripts(config, "DPkg::Pre-Invoke")?;
	apt_hook_with_pkgs(config, &changed, "DPkg::Pre-Install-Pkgs")?;

	config.apt.set("Dpkg::Use-Pty", "0");

	dpkg::run_install(cache, config)?;

	run_scripts(config, "DPkg::Post-Invoke")
}
